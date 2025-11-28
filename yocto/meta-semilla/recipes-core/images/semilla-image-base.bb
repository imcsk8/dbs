SUMMARY = "A minimal base image for the Semilla distribution"
DESCRIPTION = "Includes core utilities and branding."
LICENSE = "MIT"

# Inherit the core-image class, which provides the basic framework
inherit core-image

# Image features add functionality to the rootfs.
# 'ssh-server-openssh' will include and enable an SSH server.
#IMAGE_FEATURES += ""

# IMAGE_INSTALL specifies the packages to be installed into the rootfs.
# The '+= ' syntax appends to the list provided by the 'core-image' class.
IMAGE_INSTALL += " \
    autoconf         \
    automake         \
    bash             \
    bash-completion  \
    binutils         \
    coreutils        \
    cmake            \
    dnf              \
    dwz              \
    elfutils         \
    findutils        \
    gawk             \
    gcc              \
    gcc-symlinks     \
    grep             \
    help2man         \
    libunwind        \
    make             \
    perl             \
    python3          \
    rpm              \
    rpm-build        \
    rpm-dev          \
    rpmdevtools      \
    sed              \
    semilla-branding \
    patch            \
    python3-python-utils      \
    python3-typing-extensions \
    python3-progressbar2      \
    python3-requests          \
    python3-rpm               \
    "

# This function will run after all packages are installed into the rootfs.
# The ${IMAGE_ROOTFS} variable points to the root directory of your new image.
create_rpm_tool_symlinks() {
    echo "Creating symlinks for RPM tools in ${IMAGE_ROOTFS}${bindir}"
    # Use -r to avoid errors if a link already exists from another package
    ln -fsr ${IMAGE_ROOTFS}/bin/* ${IMAGE_ROOTFS}${bindir}/
    # rpmbuild needs 'true' to be in /bin/true
    ln -s /usr/bin/true.coreutils ${IMAGE_ROOTFS}/true
}

# Add our function to the list of commands to run at the end of rootfs creation.
ROOTFS_POSTPROCESS_COMMAND += "create_rpm_tool_symlinks;"
