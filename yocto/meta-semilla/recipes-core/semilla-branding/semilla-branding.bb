SUMMARY = "Branding package for the semilla distro"
LICENSE = "MIT"
LIC_FILES_CHKSUM = "file://${COMMON_LICENSE_DIR}/MIT;md5=0835ade698e0bcf8506ecda2f7b4f302"

# Location of the source code. 'file://' points to the 'files' subdirectory.
SRC_URI = "           \
    file://os-release \
"

# Add files to main package
# PN defines the list of files and directories that go into the 
# main package for your recipe
FILES:${PN} += "/usr/lib/os-release"

# The compilation step is handled automatically by the default do_compile task,
# which calls 'gcc hello.c -o hello'.

# This function describes how to install the compiled binary into the target rootfs.
do_install() {
    echo "***********************************************"
    echo "*                                             *"
    echo "*           Adding branding files             *"
    echo "*                                             *"
    echo "***********************************************"

    install -d ${D}/usr/lib
    install -d ${D}/etc
    install -m 0755 ${WORKDIR}/sources/os-release ${D}/usr/lib/os-release
    ln -sf ${INSTALLDIR}/usr/lib/os-release ${D}/${sysconfdir}/os-release
}
