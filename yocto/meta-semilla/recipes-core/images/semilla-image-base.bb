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
    gcc              \
    help2man         \
    perl             \
    rpm              \
    rpm-build        \
    rpmdevtools      \
    python3           \
    semilla-branding \
    "
