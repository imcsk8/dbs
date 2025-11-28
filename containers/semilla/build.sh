#!/bin/bash

# Creates a container image from a buildroot rootfs
# Iván Chavero <ichavero@chavero.com.mx>

# TODO: buildroot process

set -o errexit

if [[ "${1}" == "" ]]; then
    echo "Missing root filesystem"
    exit
fi

export ROOT_IMAGE=$1
export CONTAINER_IMAGE="ghcr.io/imcsk8/semilla" # TODO: make this a parameter?
IMAGE_VERSION="0.1.0" # TODO: make this a parameter?

echo "Creating seed 'semilla' container image using: ${ROOT_IMAGE}"

export CONTAINER=$(buildah from scratch)
echo "Entering unshare session"
buildah unshare bash <<'EOF'

echo "Container dir: $CONTAINER"
SCRATCHMOUNT=$(buildah mount $CONTAINER)

echo "Extracting root image: ${ROOT_IMAGE}"
pushd ${SCRATCHMOUNT}
if [[ ${ROOT_IMAGE} =~ \.gz$ ]]; then
    tar -zxf ${ROOT_IMAGE}
elif [[ ${ROOT_IMAGE} =~ \.zstd$ ]]; then
    tar --zstd -xf ${ROOT_IMAGE}
else
    tar -xf ${ROOT_IMAGE}
fi
popd

buildah commit ${CONTAINER} ${CONTAINER_IMAGE}:latest
EOF
echo "Exiting unshare session"

echo "Uploading container image"
podman tag $CONTAINER_IMAGE:latest  $CONTAINER_IMAGE_$IMAGE_VERSION
#podman push $CONTAINER_IMAGE:latest
#podman push $CONTAINER_IMAGE_$IMAGE_VERSION
