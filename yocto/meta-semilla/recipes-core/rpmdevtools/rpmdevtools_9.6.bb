SUMMARY = "This package contains scripts and Emacs support files to aid in development of RPM packages."
SECTION = "devel"
LICENSE = "GPL-2.0-or-later"
LIC_FILES_CHKSUM = "file://COPYING;md5=751419260aa954499f7abaabaa882bbe"
RPMDEVTOOLS_SOURCE = "rpmdevtools-RPMDEVTOOLS_9_6.tar.gz"
RPMDEVTOOLS_SITE = "https://pagure.io/rpmdevtools/archive/RPMDEVTOOLS_9_6"

SRC_URI = "${RPMDEVTOOLS_SITE}/${RPMDEVTOOLS_SOURCE}"
SRC_URI[sha256sum] = "245f847e575beb0ff9196293157c4bff9126e0d22adcf950f325f37333f88209"

S = "${UNPACKDIR}/${PN}-RPMDEVTOOLS_9_6"

DEPENDS = "bash-completion-native help2man-native perl-native python3"
RDEPENDS:${PN} += " bash python3 "

do_install() {
    sed -i 's|^#!/usr/bin/python$|#!/usr/bin/python3|' ${AUTOTOOLS_AUXDIR}/rpmdev-vercmp
}

inherit autotools
inherit pkgconfig
