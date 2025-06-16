#!/bin/bash

# Syncs the CentOS rpm group (id: 8794173) gitlab repos.
# You can sync any group givent its GROUP_ID

# NOTE: You need to export the gitlab token to the GITLAB_TOKEN environment variable

source distgit.shlib
source distgit.conf

function usage() {
cat << EOF
usage: distgit_repos clone|pull

Repository management

Subcommands:

clone Clone repos
pull  Pull repos

EOF
exit 0
}



if [[ $1 == "" ]]; then
    usage
    exit 0
fi

DRY_RUN="false"

# Get the JWT authentication token
get_token

case $1 in
      clone)
            clone_repos
        ;;
      pull)
            pull_repos
        ;;
        *) usage
esac

