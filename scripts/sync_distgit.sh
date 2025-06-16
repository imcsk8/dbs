#!/bin/bash

# Syncs the CentOS rpm group (id: 8794173) gitlab repos.
# You can sync any group givent its GROUP_ID

# NOTE: You need to export the gitlab token to the GITLAB_TOKEN environment variable

GROUP_ID=8794173

GROUP_URI="https://gitlab.com/api/v4/groups/${GROUP_ID}/projects"
CENTOS_DISTGIT_INFO="../repo_info/groups/${GROUP_ID}"
# Group metadata
GROUP_INFO="${CENTOS_DISTGIT_INFO}/repo_info.txt"

# True if there are new metadata in the repo, false any other way
REFRESH="false"
TOTAL_REPOS=0

# Checks if the group repo metadata has changed
# x-total shows the total of repos in the group
function get_group_info {

    if [[ ! -d $CENTOS_DISTGIT_INFO ]]; then
        mkdir -p $CENTOS_DISTGIT_INFO
    fi

    echo "Getting rpm group metadata"
    tmp_repo_info="${CENTOS_DISTGIT_INFO}/tmp_repo_info.txt"

    # Initialize repo_info.txt
    if [[ ! -f ${GROUP_INFO} ]]; then
        tmp_repo_info=$GROUP_INFO
    fi

    echo "curl -s --head --header PRIVATE-TOKEN: ${GITLAB_TOKEN} ${GROUP_URI}?per_page=1"

    curl -s --head --header "PRIVATE-TOKEN: ${GITLAB_TOKEN}" \
        "${GROUP_URI}?per_page=1" > ${tmp_repo_info}
    current_package_count=$(grep -Po 'x-total: \K(\d+)' $GROUP_INFO)
    tmp_package_count=$(grep -Po 'x-total: \K(\d+)' $tmp_repo_info)

    # For now we just check if package count has increased
    if (( $tmp_package_count > $current_package_count )); then
        echo "Updating group info file"
        cp $tmp_repo_info $GROUP_INFO
        REFRESH="true"
    fi
}

function get_total_repos {
    TOTAL_REPOS=$(grep -Po 'x-total: \K\d*' $GROUP_INFO)
}

# Obtiene información de los repositorios y los guarda en un archivo por página
function get_all_repo_info {
    echo "Getting CentOS Stream gitlab repos"
    PAGE=1

    # Guardamos la información de cada repo en un archivo
    while [ 1 ]; do
        echo "Getting page: ${PAGE}"
        current_page="${CENTOS_DISTGIT_INFO}/centos_distgit_page_${PAGE}.json"
        curl -s --header "PRIVATE-TOKEN: ${GITLAB_TOKEN}" \
            "${GROUP_URI}?per_page=100&page=${PAGE}&with_shared=false" | \
            jq '.[]' > $current_page
        ((PAGE++))
        if [[ ! -s $current_page ]]; then
            break
        fi
    done
    jq -s '.' ${CENTOS_DISTGIT_INFO}/centos_distgit_page_* > CENTOS_DISTGIT.json
}


function clone_repos {
    jq -r '.[] | .ssh_url_to_repo' "${CENTOS_DISTGIT_INFO}/CENTOS_DISTGIT.json"
}

echo "Getting group metadata"
get_group_info

if [[ ${REFRESH} == "true" ]]; then
    echo "Getting all the repos"
    #get_all_repo_info
    echo "Refreshing repos"
fi

