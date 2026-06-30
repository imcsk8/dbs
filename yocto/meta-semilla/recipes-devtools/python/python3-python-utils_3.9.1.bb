SUMMARY = "Collection of small Python functions and classes"
DESCRIPTION = "Python Utils is a collection of small Python functions and classes which make common patterns shorter and easier. It is by no means a complete collection but it has served quite a bit in a number of projects."
HOMEPAGE = "https://github.com/WoLpH/python-utils"
AUTHOR = "Rick van Hattem"
LICENSE = "MIT"
LIC_FILES_CHKSUM = "file://LICENSE;md5=f390846164a454d58ecb4f29c070a4b8"

PYPI_PACKAGE = "python_utils"

SRC_URI = "https://files.pythonhosted.org/packages/source/p/python_utils/python_utils-${PV}.tar.gz;downloadfilename=python_utils-${PV}.tar.gz"
SRC_URI[sha256sum] = "eb574b4292415eb230f094cbf50ab5ef36e3579b8f09e9f2ba74af70891449a0"

# Inherit from pypi and setuptools3 to handle fetching and building
inherit pypi setuptools3


