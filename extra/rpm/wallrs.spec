Name:           wallrs
Version:        1.0.0
Release:        1%{?dist}
Summary:        High-performance Wayland live wallpaper daemon and CLI controller

License:        MIT OR Apache-2.0
URL:            https://github.com/asdrome/wallrs
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz

# Build dependencies (required only during RPM compilation)
BuildRequires:  cargo
BuildRequires:  rust >= 1.85.0
BuildRequires:  clang
BuildRequires:  pkgconf
BuildRequires:  pipewire-devel
BuildRequires:  mpv-devel
BuildRequires:  wayland-devel
BuildRequires:  vulkan-loader-devel
BuildRequires:  systemd-rpm-macros

# Runtime dependencies (required for end-user execution)
Requires:       vulkan-loader
Requires:       pipewire-libs
Requires:       mpv-libs
Requires:       wayland-client

%description
A high-performance, Wayland-native live wallpaper daemon (wallrsd) and CLI
controller (wallctl) written in pure Rust with wgpu/Vulkan, PipeWire, and libmpv2.
Supports multi-layer parallax images, procedural WGSL/Shadertoy shaders,
hardware-accelerated videos, PipeWire audio reactivity, and automatic fullscreen/maximize pause.

%prep
%autosetup

%build
cargo build --release --locked

%install
install -Dm755 target/release/wallrsd %{buildroot}%{_bindir}/wallrsd
install -Dm755 target/release/wallctl %{buildroot}%{_bindir}/wallctl
install -Dm644 extra/systemd/wallrsd.service %{buildroot}%{_userunitdir}/wallrsd.service
install -Dm644 extra/metainfo/org.asdrome.wallrs.metainfo.xml %{buildroot}%{_metainfodir}/org.asdrome.wallrs.metainfo.xml
install -Dm644 LICENSE-MIT %{buildroot}%{_datadir}/licenses/%{name}/LICENSE-MIT
install -Dm644 LICENSE-APACHE %{buildroot}%{_datadir}/licenses/%{name}/LICENSE-APACHE
install -Dm644 README.md %{buildroot}%{_docdir}/%{name}/README.md

%check
cargo test --workspace --locked

%files
%license LICENSE-MIT LICENSE-APACHE
%doc README.md
%{_bindir}/wallrsd
%{_bindir}/wallctl
%{_userunitdir}/wallrsd.service
%{_metainfodir}/org.asdrome.wallrs.metainfo.xml

%changelog
* Sun Sep 06 2026 Antonio S. Dromundo <sebastiandromundo@outlook.com> - 0.1.0-1
- Initial release of wallrs live wallpaper daemon

