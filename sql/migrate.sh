#!/bin/bash

# Poor man's migration
# Iván Chavero <ichavero@chavero.com.mx>

set -xe

if [[ ! -v DATABASE_URL ]]; then
    echo "Missing DATABASE_URL variable"
    exit
fi

if [[ $1 == "" ]]; then
    echo "Usage $0 <up|down>"
    exit
fi

ACTION=$1


echo "Executing schema_${ACTION}.sql"

psql $DATABASE_URL  < schema_${ACTION}.sql
