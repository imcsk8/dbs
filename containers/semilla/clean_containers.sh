#!/bin/bash

echo "Cleaning semilla containers and image"
podman rm $(podman ps -a | grep semi | cut -d ' ' -f1)
podman rmi $(podman images | grep semi | cut -d ' ' -f1)

