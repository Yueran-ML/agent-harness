# Loaded by CMake after the flags of the compiler and the platform are set (see
# msvc-static-crt.cmake): /MD becomes /MT in the default flags of every configuration.
foreach(lang C CXX)
  foreach(config DEBUG RELEASE RELWITHDEBINFO MINSIZEREL)
    if(DEFINED CMAKE_${lang}_FLAGS_${config}_INIT)
      string(REPLACE "/MDd" "/MTd" CMAKE_${lang}_FLAGS_${config}_INIT "${CMAKE_${lang}_FLAGS_${config}_INIT}")
      string(REPLACE "/MD" "/MT" CMAKE_${lang}_FLAGS_${config}_INIT "${CMAKE_${lang}_FLAGS_${config}_INIT}")
    endif()
  endforeach()
endforeach()
