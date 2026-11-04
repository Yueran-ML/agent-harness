// A thin C interface over Signalsmith Stretch (vendor/signalsmith-stretch, MIT), for the Rust
// crate columbina-stretch (src/ffi.rs).
//
// A stretcher is an opaque handle that owns one `SignalsmithStretch<float, Random>`, made and
// configured (its preset Default) when the handle is made, which is the only time it takes
// memory: `reset`, `seek`, `process` and `set_transpose` take none (the vectors of the library
// are resized only within the capacity that `configure` gave them).
//
// The library draws random numbers when it stretches time by more than 2 (it randomises the
// phases). Its engine is a member of its own that `reset` does not seed again, so that the same
// sound after a reset would come out otherwise. Here the engine keeps its state in the handle
// (`Random::state`), and `cs_stretch_reset` sets it back to the seed: the same input after a
// reset gives the same output, to the bit.
//
// No function lets an exception out (each is `noexcept`): making a stretcher catches them and
// answers null; the others do not throw.
//
// The library's `reset` leaves one flag as it was: whether the next silence is the first of a run
// (`silenceFirst`). A stretcher whose last input ended in a long digital silence would then go
// into its silence mode at another moment than a new one does, and stretch the same input to
// other bits. `cs_stretch_reset` therefore feeds it one sample of sound (and asks for no output),
// which sets the flag as a new stretcher has it, and resets it again.

#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <exception>
#include <memory>
#include <new>

#include "signalsmith-stretch/signalsmith-stretch.h"

namespace
{
// The state of the random numbers of the stretcher being made, handed to its engine when the
// library constructs it (its constructor takes only a seed).
thread_local std::uint64_t *pending_state = nullptr;

// An engine of random numbers (SplitMix64) whose state lives in the handle, outside the
// library's object, so that a reset can seed it again.
struct Random
{
    using result_type = std::uint32_t;

    explicit Random(long) : state(pending_state) {}

    static constexpr result_type min() { return 0; }
    static constexpr result_type max() { return 0xffffffffu; }

    result_type operator()()
    {
        std::uint64_t z = (*state += 0x9e3779b97f4a7c15ull);
        z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ull;
        z = (z ^ (z >> 27)) * 0x94d049bb133111ebull;
        return static_cast<result_type>((z ^ (z >> 31)) >> 32);
    }

    std::uint64_t *state;
};

using Stretch = signalsmith::stretch::SignalsmithStretch<float, Random>;
} // namespace

struct cs_stretch
{
    explicit cs_stretch(std::uint64_t seed)
        : seed(seed), state(seed), stretch((pending_state = &state, 0L))
    {
        pending_state = nullptr;
    }

    std::uint64_t seed;
    std::uint64_t state;
    Stretch stretch;
    int channels = 0;
};

extern "C" {

/// The most channels of a stretcher (columbina-stretch's MAX_CHANNELS).
constexpr int MAX_CHANNELS = 24;

cs_stretch *cs_stretch_new(int channels, float rate, std::uint64_t seed) noexcept
{
    if (channels < 1 || channels > MAX_CHANNELS || !(rate > 0))
    {
        return nullptr;
    }
    try
    {
        std::unique_ptr<cs_stretch> made(new cs_stretch(seed));
        made->stretch.presetDefault(channels, rate);
        made->channels = channels;
        made->state = made->seed;
        return made.release();
    }
    catch (...)
    {
        return nullptr;
    }
}

void cs_stretch_free(cs_stretch *stretch) noexcept
{
    delete stretch;
}

int cs_stretch_channels(const cs_stretch *stretch) noexcept
{
    return stretch->channels;
}

int cs_stretch_block_samples(const cs_stretch *stretch) noexcept
{
    return stretch->stretch.blockSamples();
}

int cs_stretch_interval_samples(const cs_stretch *stretch) noexcept
{
    return stretch->stretch.intervalSamples();
}

int cs_stretch_input_latency(const cs_stretch *stretch) noexcept
{
    return stretch->stretch.inputLatency();
}

int cs_stretch_output_latency(const cs_stretch *stretch) noexcept
{
    return stretch->stretch.outputLatency();
}

int cs_stretch_seek_length(const cs_stretch *stretch) noexcept
{
    return stretch->stretch.seekLength();
}

void cs_stretch_reset(cs_stretch *stretch) noexcept
{
    stretch->stretch.reset();
    // One sample of sound, no output: the flag of the first silence as a new stretcher has it.
    float one = 1.0f, none = 0.0f;
    const float *inputs[MAX_CHANNELS];
    float *outputs[MAX_CHANNELS];
    for (int c = 0; c < MAX_CHANNELS; ++c)
    {
        inputs[c] = &one;
        outputs[c] = &none;
    }
    stretch->stretch.process(inputs, 1, outputs, 0);
    stretch->stretch.reset();
    stretch->state = stretch->seed;
}

void cs_stretch_set_transpose(cs_stretch *stretch, float factor) noexcept
{
    stretch->stretch.setTransposeFactor(factor);
}

void cs_stretch_seek(cs_stretch *stretch, const float *const *inputs, int samples, double rate) noexcept
{
    stretch->stretch.seek(inputs, samples, rate);
}

void cs_stretch_process(cs_stretch *stretch, const float *const *inputs, int input_samples,
                        float *const *outputs, int output_samples) noexcept
{
    stretch->stretch.process(inputs, input_samples, outputs, output_samples);
}

} // extern "C"

#ifdef COLUMBINA_STRETCH_COUNT_ALLOCATIONS
// For the tests (the crate's feature `count-allocations`): the allocation functions of C++
// replaced by ones that pass every call to malloc and free as the library's would, and count
// those of a thread while it is armed, so that a test can see that the stretch takes no memory
// where the sound is made. Each counter is a variable of its thread that takes no memory itself.
namespace
{
thread_local bool armed = false;
thread_local std::size_t allocations = 0;
thread_local std::size_t frees = 0;

void *allocate(std::size_t size)
{
    if (armed)
    {
        ++allocations;
    }
    if (size == 0)
    {
        size = 1;
    }
    for (;;)
    {
        if (void *memory = std::malloc(size))
        {
            return memory;
        }
        std::new_handler handler = std::get_new_handler();
        if (!handler)
        {
            throw std::bad_alloc();
        }
        handler();
    }
}

void release(void *memory) noexcept
{
    if (memory)
    {
        if (armed)
        {
            ++frees;
        }
        std::free(memory);
    }
}
} // namespace

void *operator new(std::size_t size)
{
    return allocate(size);
}

void *operator new[](std::size_t size)
{
    return allocate(size);
}

void *operator new(std::size_t size, const std::nothrow_t &) noexcept
{
    try
    {
        return allocate(size);
    }
    catch (...)
    {
        return nullptr;
    }
}

void *operator new[](std::size_t size, const std::nothrow_t &) noexcept
{
    try
    {
        return allocate(size);
    }
    catch (...)
    {
        return nullptr;
    }
}

void operator delete(void *memory) noexcept
{
    release(memory);
}

void operator delete[](void *memory) noexcept
{
    release(memory);
}

void operator delete(void *memory, std::size_t) noexcept
{
    release(memory);
}

void operator delete[](void *memory, std::size_t) noexcept
{
    release(memory);
}

void operator delete(void *memory, const std::nothrow_t &) noexcept
{
    release(memory);
}

void operator delete[](void *memory, const std::nothrow_t &) noexcept
{
    release(memory);
}

extern "C" {

int cs_alloc_arm(int on) noexcept
{
    bool was = armed;
    armed = on != 0;
    return was ? 1 : 0;
}

std::size_t cs_alloc_allocations(void) noexcept
{
    return allocations;
}

std::size_t cs_alloc_frees(void) noexcept
{
    return frees;
}

} // extern "C"
#endif
