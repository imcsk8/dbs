config_opts['root'] = 'tacos-rolling-{{ target_arch }}'

# fedora 31+ isn't mirrored, we need to run from koji
config_opts['mirrored'] = config_opts['target_arch'] != 'i686'

config_opts['chroot_setup_cmd'] = 'install @{% if mirrored %}buildsys-{% endif %}build'

config_opts['dist'] = 'tcrs'  # only useful for --resultdir variable subst
config_opts['extra_chroot_dirs'] = [ '/run/lock', ]
# TODO: Find out how to avoid using Fedora release versions
config_opts['releasever'] = '46'
config_opts['channel'] = 'stable'

config_opts['macros']['%dist'] = '.tcrs'
config_opts['macros']['%tacos'] = '1'
config_opts['macros']['%vendor'] = 'TacOS'

# https://fedoraproject.org/wiki/Changes/BuildWithDNF5
config_opts['package_manager'] = 'dnf5'

config_opts['bootstrap_image'] = 'registry.fedoraproject.org/fedora:rawhide'
config_opts['bootstrap_image_ready'] = True

config_opts['description'] = 'Tacos Rolling Release'

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
syslog_device=
install_weak_deps=0
metadata_expire=0
best=1
module_platform_id=platform:f{{ releasever }}
protected_packages=
user_agent={{ user_agent }}

{%- macro rawhide_gpg_keys() -%}
file:///usr/share/distribution-gpg-keys/fedora/RPM-GPG-KEY-fedora-$releasever-primary
{%- for version in [releasever|int, releasever|int - 1, releasever|int + 1]
%} file:///usr/share/distribution-gpg-keys/fedora/RPM-GPG-KEY-fedora-{{ version }}-primary
{%- endfor %}
{%- endmacro %}

# repos

[local]
name=local
baseurl=https://kojipkgs.fedoraproject.org/repos/rawhide/latest/$basearch/
cost=2000
enabled={{ not mirrored }}
skip_if_unavailable=False

[local-source]
name=local-source
baseurl=https://kojipkgs.fedoraproject.org/repos/rawhide/latest/src/
cost=2000
enabled=0
skip_if_unavailable=False

{% if mirrored %}
[fedora]
name=fedora
metalink=https://mirrors.fedoraproject.org/metalink?repo=rawhide&arch=$basearch
gpgkey={{ rawhide_gpg_keys() }}
gpgcheck=1
skip_if_unavailable=False

[fedora-debuginfo]
name=Fedora Rawhide - Debug
metalink=https://mirrors.fedoraproject.org/metalink?repo=rawhide-debug&arch=$basearch
enabled=0
gpgkey={{ rawhide_gpg_keys() }}
gpgcheck=1
skip_if_unavailable=False

[fedora-source]
name=fedora-source
metalink=https://mirrors.fedoraproject.org/metalink?repo=rawhide-source&arch=$basearch
gpgkey={{ rawhide_gpg_keys() }}
gpgcheck=1
enabled=0
skip_if_unavailable=False
{% endif %}

[tacos_os]
name=TacOS Linux {{ channel }}
baseurl=http://repos.tacos.org.mx/{{ channel }}/BaseOS/$basearch/os
gpgcheck=0
enabled=1

"""
