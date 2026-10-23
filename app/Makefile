# Cross-builds harbour-sailbubbles against the Sailfish SDK target, no qmake.
# SDK is the folder scripts/sdk.sh sets up.
SDK     ?= $(error SDK is not set, see README.md)
SYSROOT ?= $(SDK)/tgt-aarch64
CXX      = $(SDK)/wrap/sfos-aarch64-cxx
TOOLING  = $(SDK)/tooling
MOC      = $(TOOLING)/lib/ld-linux.so.2 --library-path $(TOOLING)/lib:$(TOOLING)/usr/lib $(TOOLING)/usr/lib/qt5/bin/moc
QTINC    = $(SYSROOT)/usr/include/qt5

CXXFLAGS = -std=c++11 -O2 -fPIC -Wall -Wextra -DQT_NO_DEBUG \
           -isystem $(QTINC) $(addprefix -isystem $(QTINC)/,QtCore QtGui QtQml QtQuick QtDBus QtContacts) \
           -isystem $(SYSROOT)/usr/include/sailfishapp
LDLIBS   = -lsailfishapp -lQt5Quick -lQt5Qml -lQt5DBus -lQt5Contacts -lQt5Gui -lQt5Core
LDFLAGS  = -L$(SYSROOT)/usr/lib64 -Wl,-rpath-link=$(SYSROOT)/usr/lib64:$(SYSROOT)/lib64

HEADERS = src/daemon.h src/chatmodel.h src/messagemodel.h src/contactnames.h src/appservice.h
SOURCES = src/main.cpp src/daemon.cpp src/chatmodel.cpp src/messagemodel.cpp src/contactnames.cpp src/appservice.cpp
OBJECTS = $(SOURCES:src/%.cpp=build/%.o) $(HEADERS:src/%.h=build/moc_%.o)

build/harbour-sailbubbles: $(OBJECTS)
	$(CXX) $(LDFLAGS) -o $@ $^ $(LDLIBS)

build/moc_%.cpp: src/%.h | build
	$(MOC) $< -o $@

build/%.o: src/%.cpp $(HEADERS) | build
	$(CXX) $(CXXFLAGS) -c $< -o $@

build/moc_%.o: build/moc_%.cpp
	$(CXX) $(CXXFLAGS) -Isrc -c $< -o $@

build:
	mkdir -p build

clean:
	rm -rf build

.PHONY: clean
.SECONDARY:
