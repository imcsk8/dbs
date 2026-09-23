# CentOS Stream Dependency Graph & Compilation Order

This document details the dependency graph, cycles, and parallel compilation layers computed for all **1952** source packages.

---

## 1. Summary Statistics

* **Total Source Packages:** 1952
* **Total Dependency Edges:** 14117
* **Total Strongly Connected Components (Cycles):** 1453
* **Cyclic Components (Size > 1):** 6
* **Total Compilation Layers:** 11

---

## 2. Compilation Layers Breakdown (First 5 Layers)

Below are the packages in the first 5 compilation layers:

### Layer 0 (211 packages)
adwaita-icon-theme, alsa-sof-firmware, atinject, basesystem, centos-indexhtml, centos-stream-release, cim-schema, cloud-utils, color-filesystem, container-tools, crontabs, docbook5-style-xsl, fontawesome4-fonts, fonts-rpm-macros, geolite2, google-carlito-fonts, google-crosextra-caladea-fonts, google-droid-fonts, google-noto-fonts, google-noto-sans-cjk-vf-fonts, google-roboto-slab-fonts, hamcrest, hunspell-af, hunspell-ak, hunspell-am, hunspell-ar, hunspell-as, hunspell-ast, hunspell-az, hunspell-be, hunspell-ber, hunspell-bg, hunspell-bn, hunspell-br, hunspell-ca, hunspell-cop, hunspell-csb, hunspell-cv, hunspell-cy, hunspell-da, hunspell-dsb, hunspell-el, hunspell-eo, hunspell-es, hunspell-et, hunspell-eu, hunspell-fa, hunspell-fo, hunspell-fr, hunspell-fur, ... and 161 more.

### Layer 1 (465 packages)
PyYAML, SDL2, acl, alsa-lib, annobin, apr, apr-util, asciidoc, at-spi2-core, attr, audit, autoconf, autoconf-archive, automake, avahi, bash, bash-completion, bc, bind, binutils, bison, boost, bpftool, brotli, byacc, bzip2, c-ares, ca-certificates, cairo, cdi-api, check, checkpolicy, chrpath, cmake, coreutils, cpio, crypto-policies, cscope, cups, curl, cyrus-sasl, dbus, dbus-python, debugedit, desktop-file-utils, device-mapper-persistent-data, diffutils, docbook-dtds, docbook-style-xsl, dos2unix, ... and 415 more.

### Layer 2 (875 packages)
CUnit, HdrHistogram_c, Judy, ModemManager, NetworkManager-libreswan, PackageKit, SDL3, Xaw3d, a52dec, aardvark-dns, accel-config, accountsservice, acpid, adcli, adobe-mappings-cmap, adobe-mappings-pdf, aide, alsa-firmware, alsa-plugins, alsa-tools, alsa-utils, anaconda, ansible-core, anthy-unicode, appstream-data, at, augeas, authselect, azure-vm-utils, babeltrace, baobab, bcc, bind-dyndb-ldap, biosdevname, blktrace, bluez, bolt, boom-boot, bpftrace, brltty, bubblewrap, c2esp, cachefilesd, cairomm1.16, catatonit, centos-logos, certmonger, chan, chkconfig, chrony, ... and 825 more.

### Layer 3 (233 packages)
389-ds-base, OpenIPMI, WALinuxAgent, ansible-collection-microsoft-sql, ansible-collection-redhat-leapp, ansible-freeipa, ansible-pcp, ant, antlr, aopalliance, apache-commons-cli, apache-commons-codec, apache-commons-compress, apache-commons-io, apache-commons-lang3, apache-commons-logging, apache-commons-net, assertj-core, bcel, brasero, bsf, buildah, butane, byte-buddy, cloud-init, cockpit, convmv, corosync, cracklib, crash, createrepo_c, criu, cyrus-imapd, delve, dhcpcd, dnf, dnsmasq, dyninst, efivar, flashrom, flatpak, foomatic, freeradius, frr, fwupd, gcc-toolset-15-binutils, git-lfs, gjs, gnome-bluetooth, gnome-connections, ... and 183 more.

### Layer 4 (108 packages)
NetworkManager, awscli2, bootc, byteman, ceph, clevis, cockpit-image-builder, cockpit-session-recording, dnf-plugins-core, ecj, edk2, efibootmgr, enchant2, evolution-data-server, fence-agents, firefox, gcc-toolset-15-gcc, gdm, geoclue2, geocode-glib, gnome-calculator, gnome-control-center, gnome-extensions-app, gnome-online-accounts, gnome-software, grub2, gvfs, hplip, hspell, hunspell-de, hunspell-fj, hunspell-ga, hunspell-ko, hunspell-mt, hunspell-ny, hunspell-or, hunspell-sc, hunspell-shs, hunspell-sw, hyphen-cy, hyphen-eu, hyphen-fa, hyphen-grc, hyphen-hsb, hyphen-ia, hyphen-lt, hyphen-mn, hyphen-ru, hyphen-sa, hyphen-tk, ... and 58 more.

---

## 3. SQL Insert Statements File

The complete SQL transaction to clear and populate the `package` and `package_dependencies` tables is located at:
👉 **[package_inserts.sql](scripts/package_inserts.sql)**

To execute this against your PostgreSQL database:
```bash
psql "${DATABASE_URL}" -f scripts/package_inserts.sql
```
