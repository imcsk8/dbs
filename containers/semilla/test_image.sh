#!/bin/bash

podman run --security-opt seccomp=unconfined      \
    --runtime=krun                                \
    -ti -v "${HOME}/tmp/centos_repos:/repos:Z" \
    -v "${HOME}/tmp/dbs/root:/root:Z"          \
    ghcr.io/imcsk8/semilla:latest                 \
    /bin/bash
