import QtQuick 2.5
import Sailfish.Silica 1.0

// Files picked to go with the next message; tapping one takes it out again.
SilicaListView {
    id: root

    height: count > 0 ? Theme.itemSizeExtraLarge : 0
    visible: count > 0
    orientation: ListView.Horizontal
    spacing: Theme.paddingSmall
    leftMargin: Theme.horizontalPageMargin
    rightMargin: Theme.horizontalPageMargin
    clip: true

    delegate: BackgroundItem {
        readonly property bool image: model.mime.indexOf("image/") === 0

        width: root.height
        height: root.height
        onClicked: root.model.remove(index)

        Image {
            anchors.fill: parent
            visible: image
            source: image ? "file://" + model.path : ""
            autoTransform: true
            sourceSize.width: width
            sourceSize.height: height
            fillMode: Image.PreserveAspectCrop
            asynchronous: true
            clip: true
        }

        Column {
            visible: !image
            anchors.centerIn: parent
            width: parent.width - 2 * Theme.paddingSmall

            Icon {
                anchors.horizontalCenter: parent.horizontalCenter
                source: model.mime.indexOf("video/") === 0 ? "image://theme/icon-m-file-video"
                      : model.mime.indexOf("audio/") === 0 ? "image://theme/icon-m-file-audio"
                      : "image://theme/icon-m-file-other"
            }

            Label {
                width: parent.width
                text: model.name
                font.pixelSize: Theme.fontSizeExtraSmall
                truncationMode: TruncationMode.Fade
                horizontalAlignment: Text.AlignHCenter
            }
        }

        Icon {
            anchors {
                top: parent.top
                right: parent.right
                margins: Theme.paddingSmall
            }
            source: "image://theme/icon-s-clear-opaque-background"
            color: Theme.rgba(Theme.overlayBackgroundColor, Theme.opacityHigh)

            Icon {
                anchors.centerIn: parent
                source: "image://theme/icon-s-clear-opaque-cross"
            }
        }
    }
}
