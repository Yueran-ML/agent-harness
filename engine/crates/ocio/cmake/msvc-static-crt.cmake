# The static C runtime of Microsoft (/MT) for OCIO and every dependency that its CMake builds,
# as the engine links it (engine/.cargo/config.toml: target-feature=+crt-static).
#
# Projects that ask for CMake 3.15 or later take the runtime from CMAKE_MSVC_RUNTIME_LIBRARY;
# older ones (Imath, expat, minizip-ng, pystring, as OCIO builds them) have /MD in their default
# flags, which the rules below make /MT once CMake has set them.
set(CMAKE_POLICY_DEFAULT_CMP0091 NEW)
set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded" CACHE STRING "" FORCE)
set(CMAKE_USER_MAKE_RULES_OVERRIDE "${CMAKE_CURRENT_LIST_DIR}/msvc-static-crt-rules.cmake")

# expat sets its runtime itself, from an option of its own (and then names its library
# libexpatMT rather than libexpatMD).
set(EXPAT_MSVC_STATIC_CRT ON CACHE BOOL "" FORCE)
