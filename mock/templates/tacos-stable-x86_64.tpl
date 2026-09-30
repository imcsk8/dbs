config_opts['root'] = 'tacos-stable-x86_64-{{ target_arch }}'
config_opts['chroot_setup_cmd'] = 'install @buildsys-build'
config_opts['dist'] = '.tcrs'
config_opts['releasever'] = 'rawhide'
config_opts['package_manager'] = 'dnf5'
config_opts['bootstrap_image'] = 'registry.fedoraproject.org/fedora:rawhide'
config_opts['bootstrap_image_ready'] = True
config_opts['description'] = 'Custom Distribution Chroot for tacos-stable-x86_64'

config_opts['macros']['%dist'] = '.tcrs'
config_opts['macros']['%vendor'] = 'tacos-stable-x86_64'
config_opts['macros']['%_smp_mflags'] = '-j24'
config_opts['macros']['%_smp_build_ncpus'] = '24'
config_opts['macros']['%_smp_ncpus_max'] = '24'

config_opts['dnf.conf'] = """
[main]
keepcache=1
system_cachedir=/var/cache/dnf
debuglevel=2
reposdir=/dev/null
logfile=/var/log/yum.log
retries=20
obsoletes=1
gpgcheck=0
assumeyes=1
syslog_ident=mock
install_weak_deps=0
best=1

[tacos-staging]
name=TacOS Dynamic Staging Repository
baseurl=file:///srv/dbs/tacos/staging/rpms/{{ target_arch }}
enabled=1
gpgcheck=0
metadata_expire=0
cost=1
priority=1
skip_if_unavailable=1

[tacos-local]
name=TacOS Local Build Repository
baseurl=file:///srv/dbs/tacos/distro/tacos-stable-x86_64/{{ target_arch }}
enabled=1
gpgcheck=0
metadata_expire=0
cost=1
priority=2
skip_if_unavailable=1

[fedora]
name=Fedora Rawhide
metalink=https://mirrors.fedoraproject.org/metalink?repo=rawhide&arch=$basearch
gpgcheck=0
enabled=1
priority=99
"""

# Build isolation and execution tuning: disable tmpfs to avoid disk-full errors on massive packages (GCC, LLVM, etc.)
config_opts['plugin_conf']['tmpfs_enable'] = False

import os
if os.path.exists('/srv/dbs/mock'):
    config_opts['basedir'] = '/srv/dbs/mock'

# Grant capabilities and relax seccomp for low-level system testing (ptrace, sched, vmsplice)
config_opts['seccomp'] = False
config_opts['nspawn_args'] += ['--capability=CAP_SYS_PTRACE,CAP_SYS_ADMIN']
