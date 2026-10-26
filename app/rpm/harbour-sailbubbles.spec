Name:       harbour-sailbubbles
Version:    0.3.0
Release:    1
Summary:    SailBubbles: iMessage for Sailfish OS
License:    SSPL-1.0
Source0:    harbour-sailbubbles.tar
Requires:   sailfishsilica-qt5
Requires:   sailbubbles-daemon >= 0.2.0
Obsoletes:  harbour-openbubbles < 0.2.0

%define debug_package %{nil}

%description
SailBubbles, the Silica app for sailbubbles-daemon: chats, conversations and sending.

%prep

%build

%install
mkdir -p %{buildroot}
tar -xf %{SOURCE0} -C %{buildroot}

%files
/usr/bin/harbour-sailbubbles
/usr/share/harbour-sailbubbles
/usr/share/applications/harbour-sailbubbles.desktop
/usr/share/icons/hicolor/*/apps/harbour-sailbubbles.png
/usr/share/dbus-1/services/org.sailbubbles.harbour-sailbubbles.service
