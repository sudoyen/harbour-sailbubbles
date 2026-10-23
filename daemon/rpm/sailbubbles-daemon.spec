Name:       sailbubbles-daemon
Version:    0.2.0
Release:    1
Summary:    SailBubbles iMessage daemon for Sailfish OS, based on rustpush
License:    SSPL-1.0 and LGPL-3.0-or-later
Source0:    sailbubbles-daemon
Source1:    sailbubbles-daemon.service
Source2:    SailBubbles.permission
Source3:    x-sailbubbles.message.conf
# libheif and libde265 for HEIC photos, which Sailfish cannot decode
Source4:    heic-libs.tar
# the same daemon under its name from before the rename; its data moves over
Obsoletes:  openbubbles-daemon < 0.2.0

%define debug_package %{nil}

%description
Keeps the Apple push connection open and handles iMessage sign-in,
sending and receiving, through a registration relay on your own Mac.

%prep

%build

%install
install -D -m 755 %{SOURCE0} %{buildroot}%{_bindir}/sailbubbles-daemon
install -D -m 644 %{SOURCE1} %{buildroot}/usr/lib/systemd/user/sailbubbles-daemon.service
# started at login without anyone running systemctl
mkdir -p %{buildroot}/usr/lib/systemd/user/user-session.target.wants
ln -s ../sailbubbles-daemon.service %{buildroot}/usr/lib/systemd/user/user-session.target.wants/sailbubbles-daemon.service
install -D -m 644 %{SOURCE2} %{buildroot}/etc/sailjail/permissions/SailBubbles.permission
install -D -m 644 %{SOURCE3} %{buildroot}/usr/share/lipstick/notificationcategories/x-sailbubbles.message.conf
mkdir -p %{buildroot}
tar -xf %{SOURCE4} -C %{buildroot}

%files
%{_bindir}/sailbubbles-daemon
/usr/lib/systemd/user/sailbubbles-daemon.service
/usr/lib/systemd/user/user-session.target.wants/sailbubbles-daemon.service
/etc/sailjail/permissions/SailBubbles.permission
/usr/share/lipstick/notificationcategories/x-sailbubbles.message.conf
/usr/lib64/sailbubbles-daemon
/usr/share/licenses/sailbubbles-daemon
