# Distribution Build System (DBS)

Documentation and software for building  linux distributions from
a given dist-git.

There are many ways of creating Linux distributions and it appears that
each distribution has its own "secret sauce" for bootstrapping and building
the base image and I don't like that.

This project is aimed to allow users to create Linux distributions from scratch
regardless of the package system.

## Steps for creating a distribution

### Get CentOS dist-git repositories

**Get repository metadata**

```bash
scripts $ ./sync_distgit.sh
```

**Clone Repositories**
```bash
scripts $ ./distgit_repos.sh clone
```

**Pull Repositories**
**TODO**
```bash
scripts $ ./distgit_repos.sh pull
```

## CI/CD Build Pipeline

### Build packages in proper order

The `core` and `base` package groups should be built before any package in the
dist-git.

**TODO**

### Build the other packages

TODO: Automate this task 

### Build Images

TODO: Create pipeline modules for building ISO, qcow and container images

