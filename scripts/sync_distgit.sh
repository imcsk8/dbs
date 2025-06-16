#!/bin/bash

# Syncs the CentOS rpm group (id: 8794173) gitlab repos.
# You can sync any group givent its GROUP_ID

# NOTE: You need to export the gitlab token to the GITLAB_TOKEN environment variable

source distgit.shlib
source distgit.conf

FORCE_INIT="false"

if [[ $1 == "--force-init" ]]; then
    FORCE_INIT="true"
fi

GROUP_ID=8794173

GROUP_URI="https://gitlab.com/api/v4/groups/${GROUP_ID}/projects"
CENTOS_DISTGIT_PATH="/srv/dbs/repo_info/groups/${GROUP_ID}"
# Group metadata
GROUP_INFO="${CENTOS_DISTGIT_PATH}/repo_info.txt"

# True if there are new metadata in the repo, false any other way
REFRESH="false"
TOTAL_REPOS=0

echo "Getting group metadata"
get_group_info

if [[ ${REFRESH} == "true" || ${FORCE_INIT} == "true" ]]; then
    echo "Getting all the repos"
    get_all_repo_info
    echo "Refreshing repos"
fi

