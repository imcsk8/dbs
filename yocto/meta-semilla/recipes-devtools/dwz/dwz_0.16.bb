DESCRIPTION = "DWZ - DWARF optimization and duplicate removal tool"
HOMEPAGE = "https://sourceware.org/dwz/"
LICENSE = "GPL-3.0-or-later"
LIC_FILES_CHKSUM = "file://COPYING3;md5=d32239bcb673463ab874e80d47fae504"

inherit autotools

SRC_URI = "https://sourceware.org/pub/dwz/releases/dwz-${PV}.tar.xz"
SRC_URI[sha512sum] = "1d6584ad8a0558b8a1351472c968c7b86849267872a50f11ab7b2a0866403dfb2dcf5e14c7c1a97a7c014c7b9fe8ce56c37aaa28b04bd6ddb0fd04bfbc8b97fe"

S = "${UNPACKDIR}/${PN}"
FILES:${PN} += "${bindir}/dwz"

DEPENDS = "elfutils xxhash"

# dwz is known to embed build paths in its debug information.
# This tells the Yocto QA checks to ignore this specific issue for this package.
INSANE_SKIP:${PN} += "buildpaths"
INSANE_SKIP:${PN}-dbg += "buildpaths"

