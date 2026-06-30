SUMMARY = "Backported and Experimental Type Hints for Python"
DESCRIPTION = "The typing_extensions module contains both backports of the latest \
typing features to older Python versions, and experimental new features that \
will be added to the typing module in a future Python release."
HOMEPAGE = "https://github.com/python/typing_extensions"
AUTHOR = "Python Typing Team"
LICENSE = "PSF-2.0"
LIC_FILES_CHKSUM = "file://LICENSE;md5=fcf6b249c2641540219a727f35d8d2c2"

# Inherit from pypi and python_pep517 to handle pyproject.toml based builds
inherit pypi python_pep517

PYPI_PACKAGE = "typing_extensions"
SRC_URI[sha256sum] = "0cea48d173cc12fa28ecabc3b837ea3cf6f38c6d1136f85cbaaf598984861466"

# Build dependencies needed for the PEP517 backend
DEPENDS += "python3-setuptools-native"
