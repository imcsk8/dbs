#!/bin/bash

podman run --security-opt seccomp=unconfined      \
    --runtime=krun                                \
    -ti -v /home/imcsk8/tmp/centos_repos:/repos:Z \
    -v /home/imcsk8/tmp/dbs/root:/root:Z          \
    ghcr.io/imcsk8/semilla:latest                 \
    /bin/bash
