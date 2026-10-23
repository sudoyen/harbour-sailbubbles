import QtQuick 2.0
import Sailfish.Silica 1.0
import Sailfish.Gallery 1.0

FullscreenContentPage {
    id: page

    property string path
    property string mime

    allowedOrientations: Orientation.All

    GalleryMediaPlayer {
        id: player
        source: "file://" + page.path
        autoPlay: true
    }

    GalleryVideoOutput {
        anchors.fill: parent
        player: player
    }

    MouseArea {
        anchors.fill: parent
        onClicked: player.togglePlay()
    }

    Icon {
        anchors.centerIn: parent
        visible: !player.playing
        source: "image://theme/icon-m-play"
    }

    SaveButton {
        path: page.path
        mime: page.mime
    }
}
