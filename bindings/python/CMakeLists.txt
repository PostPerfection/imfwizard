# Python bindings for IMF Wizard
# These are subprocess-based wrappers around the CLI binary.
# No compilation needed — just install the Python package.
#
# Usage:
#   pip install -e bindings/python/
#   or: IMFWIZARD_BIN=/path/to/imfwizard python -c "import imfwizard; ..."

target_link_libraries(pyimfwizard PRIVATE
  imfwizard_core
  Python3::Python
)

set_target_properties(pyimfwizard PROPERTIES
  OUTPUT_NAME "_imfwizard"
  PREFIX ""
)

# Let CMake/SWIG handle the suffix properly
if(Python3_SOABI)
  set_target_properties(pyimfwizard PROPERTIES SUFFIX ".${Python3_SOABI}.so")
endif()

# Install the Python module
install(TARGETS pyimfwizard DESTINATION ${Python3_SITEARCH})
install(FILES ${CMAKE_CURRENT_BINARY_DIR}/imfwizard.py DESTINATION ${Python3_SITEARCH})
