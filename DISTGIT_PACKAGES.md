# Fedora Dist-Git Source Packages vs. KIWI-NG Binary Packages

## 1. Overview & Architectural Distinction

When working between **KIWI-NG** (image building) and **DBS** (dist-git synchronization & Mock package building), there is a fundamental distinction between **Binary RPM Packages** and **Source RPM (SRPM) Dist-Git Repositories**:

* **KIWI-NG (`config.xml`):** Declares **Binary RPMs** to be installed into the root filesystem image via `dnf5` (e.g. `<package name="gnome-shell-extension-user-theme"/>`).
* **Fedora Dist-Git (`src.fedoraproject.org`):** Maintains **Source RPM repositories** only (`https://src.fedoraproject.org/rpms/<SRPM>.git`). Each repository contains a `.spec` file that compiles into one or more binary RPM subpackages.

Standalone Git repositories do **not** exist for subpackages. For example, cloning `gnome-shell-extension-user-theme` will return a 404 error because the upstream source repository is named **`gnome-shell-extensions`** (plural).

---

## 2. GNOME Shell Extensions Analysis

The following 6 packages declared in `config.xml` are binary subpackages built from the single source RPM [`gnome-shell-extensions`](https://src.fedoraproject.org/rpms/gnome-shell-extensions):

1. `gnome-shell-extension-user-theme`
2. `gnome-shell-extension-apps-menu`
3. `gnome-shell-extension-auto-move-windows`
4. `gnome-shell-extension-launch-new-instance`
5. `gnome-shell-extension-places-menu`
6. `gnome-shell-extension-drive-menu`

### Upstream Spec Definition (`gnome-shell-extensions.spec`):
```spec
Name:           gnome-shell-extensions
Version:        51.0
%global pkg_prefix gnome-shell-extension

%package -n %{pkg_prefix}-user-theme
Summary:        Support for custom themes in GNOME Shell

%package -n %{pkg_prefix}-apps-menu
Summary:        Application menu for GNOME Shell

%package -n %{pkg_prefix}-auto-move-windows
Summary:        Assign specific workspaces to applications

%package -n %{pkg_prefix}-launch-new-instance
Summary:        Always launch a new instance when clicking an app icon

%package -n %{pkg_prefix}-places-menu
Summary:        Places menu for GNOME Shell

%package -n %{pkg_prefix}-drive-menu
Summary:        Disk and removable media drive menu
```

Compiling `gnome-shell-extensions` once inside Mock produces all 6 RPMs simultaneously.

---

## 3. TacOS Full Package Mapping Matrix

Out of the **70 unique packages** declared in [`config.xml`](config.xml):
* **33 packages** are 1:1 direct Source RPM repositories in dist-git.
* **35 packages** are binary subpackages derived from **14 upstream SRPMs**.
* **2 packages** are custom TacOS packages (`tacosctl`, `tacos-os-repo`).

### Subpackage to Parent SRPM Mapping

| Upstream Dist-Git SRPM | Git Clone URL | Binary Packages in `config.xml` |
| :--- | :--- | :--- |
| **`gnome-shell-extensions`** | `https://src.fedoraproject.org/rpms/gnome-shell-extensions.git` | `gnome-shell-extension-user-theme`<br>`gnome-shell-extension-apps-menu`<br>`gnome-shell-extension-auto-move-windows`<br>`gnome-shell-extension-launch-new-instance`<br>`gnome-shell-extension-places-menu`<br>`gnome-shell-extension-drive-menu` |
| **`grub2`** | `https://src.fedoraproject.org/rpms/grub2.git` | `grub2-efi-x64`<br>`grub2-efi-x64-modules`<br>`grub2-pc`<br>`grub2-pc-modules`<br>`grub2-efi-x64-cdboot` |
| **`kernel`** | `https://src.fedoraproject.org/rpms/kernel.git` | `kernel`<br>`kernel-modules`<br>`kernel-modules-core`<br>`kernel-modules-extra` |
| **`plymouth`** | `https://src.fedoraproject.org/rpms/plymouth.git` | `plymouth`<br>`plymouth-graphics-libs`<br>`plymouth-plugin-label`<br>`plymouth-plugin-script`<br>`plymouth-plugin-two-step`<br>`plymouth-scripts`<br>`plymouth-theme-script` |
| **`glibc`** | `https://src.fedoraproject.org/rpms/glibc.git` | `glibc-langpack-en`<br>`glibc-langpack-es` |
| **`dracut`** | `https://src.fedoraproject.org/rpms/dracut.git` | `dracut-live`<br>`dracut-network` |
| **`kiwi`** | `https://src.fedoraproject.org/rpms/kiwi.git` | `dracut-kiwi-oem-dump`<br>`dracut-kiwi-oem-repart` |
| **`NetworkManager`** | `https://src.fedoraproject.org/rpms/NetworkManager.git` | `NetworkManager`<br>`NetworkManager-wifi` |
| **`fedora-repos`** | `https://src.fedoraproject.org/rpms/fedora-repos.git` | `fedora-repos-rawhide` |
| **`glib2`** | `https://src.fedoraproject.org/rpms/glib2.git` | `glib2-devel` |
| **`vim`** | `https://src.fedoraproject.org/rpms/vim.git` | `vim-enhanced` |
| **`cracklib`** | `https://src.fedoraproject.org/rpms/cracklib.git` | `cracklib-dicts` |
| **`dbus`** | `https://src.fedoraproject.org/rpms/dbus.git` | `dbus-daemon` |
| **`systemd`** | `https://src.fedoraproject.org/rpms/systemd.git` | `systemd-pam` |
| **`qemu`** | `https://src.fedoraproject.org/rpms/qemu.git` | `qemu-system-x86` |
| **`shim`** | `https://src.fedoraproject.org/rpms/shim.git` | `shim-x64` |

### Direct 1:1 Dist-Git Packages

These packages share identical Source RPM and Binary RPM names:

`bash`, `btrfs-progs`, `e2fsprogs`, `gdisk`, `htop`, `zstd`, `mbuffer`, `pv`, `kexec-tools`, `mokutil`, `efibootmgr`, `neovim`, `tmux`, `fontconfig`, `dconf`, `toolbox`, `strace`, `libpwquality`, `openssl`, `gnome-tweaks`, `flatpak`, `gnome-extensions-app`, `zenity`, `lm_sensors`, `cockpit`, `xfsprogs`, `dosfstools`, `basesystem`, `dnf5`, `filesystem`.

### Custom TacOS Packages

These are maintained locally in the [`specs/`](specs/) directory of the TacOS repository:
* `tacosctl` (installer and deployment CLI)
* `tacos-os-repo` / `tacos-release` (repository configurations and release definitions)

### Comps Groups

These entries in `config.xml` refer to DNF comps groups / environments rather than standalone RPMs:
* `@workstation-product-environment`
* `@server-product-environment`
* `<namedCollection name="core"/>`
* `<namedCollection name="system-tools"/>`

---

## 4. Minimal Source RPM Build List

To build all 70 TacOS binary packages from source using DBS, you only need to clone and build the following **48 Source RPMs**:

```text
basesystem
bash
btrfs-progs
cockpit
cracklib
dbus
dconf
dnf5
dosfstools
dracut
e2fsprogs
efibootmgr
fedora-repos
filesystem
flatpak
fontconfig
gdisk
glib2
glibc
gnome-extensions-app
gnome-shell-extensions
gnome-tweaks
grub2
htop
kernel
kexec-tools
kiwi
libpwquality
lm_sensors
mbuffer
mokutil
neovim
NetworkManager
openssl
plymouth
pv
qemu
shim
strace
systemd
tmux
toolbox
vim
xfsprogs
zenity
zstd
```

---

## 5. DBS Usage Commands

```bash
# Clone the parent source repository
./bin/dbs --config tacos.toml distgit clone gnome-shell-extensions

# Build the package in Mock (generates all subpackage RPMs into staging)
./bin/dbs --config tacos.toml build /srv/dbs/tacos/rpm/gnome-shell-extensions/gnome-shell-extensions.spec
```
