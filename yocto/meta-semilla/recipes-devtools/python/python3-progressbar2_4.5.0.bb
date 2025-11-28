SUMMARY = "Text progress bars for Python"
DESCRIPTION = "A Python Progressbar library to provide visual (yet text based) \
progress to long running operations. It is extremely easy to use, and very \
powerful. It will also automatically resize the progressbar to the terminal \
width."
HOMEPAGE = "https://github.com/WoLpH/python-progressbar"
AUTHOR = "Rick van Hattem"
LICENSE = "BSD-3-Clause"
LIC_FILES_CHKSUM = "file://LICENSE;md5=9e601ea11ed540fbf3aec2b861674b71"

# Inherit from pypi and setuptools3 to handle fetching and building
inherit pypi python_pep517

# Source URL from PyPI
SRC_URI[sha256sum] = "6662cb624886ed31eb94daf61e27583b5144ebc7383a17bae076f8f4f59088fb"

DEPENDS += "python3-setuptools-native python3-setuptools-scm-native"
# Runtime dependencies
RDEPENDS:${PN} += "python3-python-utils"
