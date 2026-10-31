// A thin C interface over OpenColorIO 2.x, for the Rust crate columbina-ocio (src/ffi.rs).
//
// Every object is an opaque handle that owns a shared pointer of OCIO's. Functions that can fail
// catch every exception, keep its message for the calling thread (ocio_last_error) and return a
// null handle or -1. Strings handed out belong to the handle they came from and live as long as
// it; the Rust side copies them at once.

#include <OpenColorIO/OpenColorIO.h>

#include <cstring>
#include <exception>
#include <sstream>
#include <string>
#include <vector>

namespace OCIO = OCIO_NAMESPACE;

namespace
{
thread_local std::string last_error;

void fail(const char * message)
{
    last_error = message ? message : "unknown error";
}

template <typename F> auto guard(F && f, decltype(f()) failed) -> decltype(f())
{
    try
    {
        last_error.clear();
        return f();
    }
    catch (const std::exception & e)
    {
        fail(e.what());
    }
    catch (...)
    {
        fail("an exception that is not a std::exception");
    }
    return failed;
}
} // namespace

struct ocio_config
{
    OCIO::ConstConfigRcPtr config;
    // Names of every colour space, active or not, scene- or display-referred, in the config's order.
    std::vector<std::string> spaces;
};

struct ocio_processor
{
    OCIO::ConstProcessorRcPtr processor;
    OCIO::ConstCPUProcessorRcPtr cpu;
    std::string cache_id;
};

struct ocio_shader
{
    OCIO::GpuShaderDescRcPtr desc;
    std::string text;
};

static ocio_config * wrap(OCIO::ConstConfigRcPtr config)
{
    auto * out = new ocio_config{config, {}};
    const int count = config->getNumColorSpaces(OCIO::SEARCH_REFERENCE_SPACE_ALL, OCIO::COLORSPACE_ALL);
    for (int i = 0; i < count; ++i)
    {
        out->spaces.emplace_back(
            config->getColorSpaceNameByIndex(OCIO::SEARCH_REFERENCE_SPACE_ALL, OCIO::COLORSPACE_ALL, i));
    }
    return out;
}

static ocio_processor * wrap(OCIO::ConstProcessorRcPtr processor)
{
    auto * out = new ocio_processor{processor, processor->getDefaultCPUProcessor(), processor->getCacheID()};
    return out;
}

extern "C" {

const char * ocio_last_error() { return last_error.c_str(); }

const char * ocio_version() { return OCIO::GetVersion(); }

// --- configs -------------------------------------------------------------------------------------

ocio_config * ocio_config_from_file(const char * path)
{
    return guard([&] { return wrap(OCIO::Config::CreateFromFile(path)); }, (ocio_config *)nullptr);
}

ocio_config * ocio_config_from_text(const char * text, const char * working_dir)
{
    return guard(
        [&] {
            std::istringstream stream(text);
            auto config = OCIO::Config::CreateFromStream(stream);
            if (working_dir && *working_dir)
            {
                auto editable = config->createEditableCopy();
                editable->setWorkingDir(working_dir);
                config = editable;
            }
            return wrap(config);
        },
        (ocio_config *)nullptr);
}

// The colour spaces of `extra` that `base` has no space of the same name for, added to a copy of
// `base`, which is then validated. The two must share their reference spaces (ACES2065-1 for
// scene-referred spaces, CIE XYZ D65 for display-referred ones), as the built-in ACES configs and
// our own fragment do.
ocio_config * ocio_config_extend(const ocio_config * base, const ocio_config * extra)
{
    return guard(
        [&] {
            auto editable = base->config->createEditableCopy();
            for (const auto & name : extra->spaces)
            {
                if (editable->getColorSpace(name.c_str()))
                {
                    std::string message = "the colour space '" + name + "' is in the base config already";
                    throw OCIO::Exception(message.c_str());
                }
                editable->addColorSpace(extra->config->getColorSpace(name.c_str()));
            }
            editable->validate();
            return wrap(editable);
        },
        (ocio_config *)nullptr);
}

void ocio_config_free(ocio_config * config) { delete config; }

int ocio_config_validate(const ocio_config * config)
{
    return guard(
        [&] {
            config->config->validate();
            return 0;
        },
        -1);
}

const char * ocio_config_name(const ocio_config * config) { return config->config->getName(); }

int ocio_config_num_spaces(const ocio_config * config) { return (int)config->spaces.size(); }

// What a colour space is: its name, family, encoding and description, whether it is
// display-referred (1) or scene-referred (0), whether it is data, and how many aliases it has.
int ocio_config_space(const ocio_config * config, int index, const char ** name, const char ** family,
                      const char ** encoding, const char ** description, int * display, int * data,
                      int * aliases)
{
    return guard(
        [&] {
            if (index < 0 || index >= (int)config->spaces.size())
            {
                throw OCIO::Exception("no colour space of that index");
            }
            auto space = config->config->getColorSpace(config->spaces[index].c_str());
            *name = space->getName();
            *family = space->getFamily();
            *encoding = space->getEncoding();
            *description = space->getDescription();
            *display = space->getReferenceSpaceType() == OCIO::REFERENCE_SPACE_DISPLAY ? 1 : 0;
            *data = space->isData() ? 1 : 0;
            *aliases = (int)space->getNumAliases();
            return 0;
        },
        -1);
}

const char * ocio_config_space_alias(const ocio_config * config, int index, int alias)
{
    return guard(
        [&] {
            auto space = config->config->getColorSpace(config->spaces.at(index).c_str());
            return space->getAlias((size_t)alias);
        },
        (const char *)nullptr);
}

// The name that `name` (a name or an alias) stands for, or null when there is none.
const char * ocio_config_canonical_name(const ocio_config * config, const char * name)
{
    return guard(
        [&] {
            auto space = config->config->getColorSpace(name);
            return space ? space->getName() : (const char *)nullptr;
        },
        (const char *)nullptr);
}

int ocio_config_num_displays(const ocio_config * config) { return config->config->getNumDisplays(); }

const char * ocio_config_display(const ocio_config * config, int index)
{
    return config->config->getDisplay(index);
}

int ocio_config_num_views(const ocio_config * config, const char * display)
{
    return guard([&] { return config->config->getNumViews(display); }, -1);
}

const char * ocio_config_view(const ocio_config * config, const char * display, int index)
{
    return guard([&] { return config->config->getView(display, index); }, (const char *)nullptr);
}

// --- processors ----------------------------------------------------------------------------------

ocio_processor * ocio_processor_spaces(const ocio_config * config, const char * src, const char * dst)
{
    return guard([&] { return wrap(config->config->getProcessor(src, dst)); }, (ocio_processor *)nullptr);
}

ocio_processor * ocio_processor_display_view(const ocio_config * config, const char * src, const char * display,
                                             const char * view, int inverse)
{
    return guard(
        [&] {
            auto transform = OCIO::DisplayViewTransform::Create();
            transform->setSrc(src);
            transform->setDisplay(display);
            transform->setView(view);
            const auto direction = inverse ? OCIO::TRANSFORM_DIR_INVERSE : OCIO::TRANSFORM_DIR_FORWARD;
            return wrap(config->config->getProcessor(transform, direction));
        },
        (ocio_processor *)nullptr);
}

ocio_processor * ocio_processor_builtin(const ocio_config * config, const char * style, int inverse)
{
    return guard(
        [&] {
            auto transform = OCIO::BuiltinTransform::Create();
            transform->setStyle(style);
            const auto direction = inverse ? OCIO::TRANSFORM_DIR_INVERSE : OCIO::TRANSFORM_DIR_FORWARD;
            return wrap(config->config->getProcessor(transform, direction));
        },
        (ocio_processor *)nullptr);
}

void ocio_processor_free(ocio_processor * processor) { delete processor; }

const char * ocio_processor_cache_id(const ocio_processor * processor) { return processor->cache_id.c_str(); }

int ocio_processor_is_noop(const ocio_processor * processor) { return processor->processor->isNoOp() ? 1 : 0; }

// `count` pixels of packed RGB, in place.
int ocio_processor_apply(const ocio_processor * processor, float * rgb, long long count)
{
    return guard(
        [&] {
            OCIO::PackedImageDesc image(rgb, (long)count, 1, 3);
            processor->cpu->apply(image);
            return 0;
        },
        -1);
}

// --- shaders -------------------------------------------------------------------------------------

// The shader of a processor in Vulkan GLSL 4.6, its names starting with `prefix`, its function
// `<prefix>main`, its 1D LUTs as 2D textures of at most `max_width` texels a row.
ocio_shader * ocio_processor_shader(const ocio_processor * processor, const char * prefix, unsigned max_width)
{
    return guard(
        [&] {
            auto desc = OCIO::GpuShaderDesc::CreateShaderDesc();
            desc->setLanguage(OCIO::GPU_LANGUAGE_GLSL_VK_4_6);
            desc->setResourcePrefix(prefix);
            const std::string function = std::string(prefix) + "main";
            desc->setFunctionName(function.c_str());
            desc->setPixelName((std::string(prefix) + "pixel").c_str());
            desc->setAllowTexture1D(false);
            desc->setTextureMaxWidth(max_width);
            desc->setDescriptorSetIndex(0, 1);
            processor->processor->getDefaultGPUProcessor()->extractGpuShaderInfo(desc);
            return new ocio_shader{desc, desc->getShaderText()};
        },
        (ocio_shader *)nullptr);
}

void ocio_shader_free(ocio_shader * shader) { delete shader; }

const char * ocio_shader_text(const ocio_shader * shader) { return shader->text.c_str(); }

int ocio_shader_num_uniforms(const ocio_shader * shader) { return (int)shader->desc->getNumUniforms(); }

int ocio_shader_num_textures(const ocio_shader * shader) { return (int)shader->desc->getNumTextures(); }

// A texture of a 1D LUT (or of a table): its names, size, channels (1 or 3), whether it is
// sampled to the nearest texel (1) or linearly (0), and its values (width × height × channels).
int ocio_shader_texture(const ocio_shader * shader, int index, const char ** name, const char ** sampler,
                        unsigned * width, unsigned * height, int * channels, int * nearest, const float ** values)
{
    return guard(
        [&] {
            OCIO::GpuShaderDesc::TextureType type;
            OCIO::GpuShaderDesc::TextureDimensions dimensions;
            OCIO::Interpolation interpolation;
            shader->desc->getTexture((unsigned)index, *name, *sampler, *width, *height, type, dimensions,
                                     interpolation);
            *channels = type == OCIO::GpuShaderDesc::TEXTURE_RED_CHANNEL ? 1 : 3;
            *nearest = interpolation == OCIO::INTERP_NEAREST ? 1 : 0;
            shader->desc->getTextureValues((unsigned)index, *values);
            return 0;
        },
        -1);
}

int ocio_shader_num_3d_textures(const ocio_shader * shader) { return (int)shader->desc->getNum3DTextures(); }

// A texture of a 3D LUT: its names, its edge, whether it is sampled to the nearest texel, and its
// values (edge³ × 3, red fastest).
int ocio_shader_3d_texture(const ocio_shader * shader, int index, const char ** name, const char ** sampler,
                           unsigned * edge, int * nearest, const float ** values)
{
    return guard(
        [&] {
            OCIO::Interpolation interpolation;
            shader->desc->get3DTexture((unsigned)index, *name, *sampler, *edge, interpolation);
            *nearest = interpolation == OCIO::INTERP_NEAREST ? 1 : 0;
            shader->desc->get3DTextureValues((unsigned)index, *values);
            return 0;
        },
        -1);
}

} // extern "C"
