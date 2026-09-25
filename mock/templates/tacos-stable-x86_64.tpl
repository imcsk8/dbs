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
config_opts['macros']['%_smp_mflags'] = '-j2'
config_opts['macros']['%_smp_build_ncpus'] = '2'
config_opts['macros']['%_smp_ncpus_max'] = '2'

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

[fedora]
name=Fedora Rawhide
metalink=https://mirrors.fedoraproject.org/metalink?repo=rawhide&arch=$basearch
gpgcheck=0
enabled=1
"""

# Build isolation and execution tuning
config_opts['plugin_conf']['tmpfs_enable'] = True
config_opts['plugin_conf']['tmpfs_opts']['required_ram_mb'] = 4096
config_opts['plugin_conf']['tmpfs_opts']['keep_mounted'] = False

# Grant capabilities and relax seccomp for low-level system testing (ptrace, sched, vmsplice)
config_opts['seccomp'] = False
config_opts['nspawn_args'] += ['--capability=CAP_SYS_PTRACE,CAP_SYS_ADMIN']
