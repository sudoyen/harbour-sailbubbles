import QtQuick 2.5
import Sailfish.Silica 1.0

// One attachment of a message: the picture itself once downloaded, else a
// tile that downloads it on tap.
Item {
    id: root

    property var attachment
    property real maxWidth
    property bool requested

    readonly property bool ready: attachment.path !== ""
    readonly property bool image: attachment.mime.indexOf("image/") === 0
    readonly property bool gif: attachment.mime === "image/gif"
    readonly property bool video: attachment.mime.indexOf("video/") === 0
    readonly property string url: "file://" + attachment.path

    signal pressAndHold
    signal openPhoto

    width: ready && image ? maxWidth : tile.width
    height: ready && image ? picture.height : tile.height

    function open() {
        if (!ready) {
            requested = daemon.download(attachment.id)
        } else if (gif) {
            pageStack.push(Qt.resolvedUrl("GalleryPage.qml"), { photos: [attachment], startIndex: 0 })
        } else if (image) {
            openPhoto()
        } else if (video) {
            pageStack.push(Qt.resolvedUrl("VideoPage.qml"), { path: attachment.path, mime: attachment.mime })
        }
    }

    Loader {
        id: picture
        active: root.ready && root.image
        width: root.maxWidth
        height: item ? item.height : 0
        sourceComponent: root.gif ? animated : still
    }

    Component {
        id: still
        Image {
            width: root.maxWidth
            // decode at display size, not at the camera's
            sourceSize.width: root.maxWidth
            height: implicitWidth > 0 ? width * implicitHeight / implicitWidth : width * 0.75
            fillMode: Image.PreserveAspectFit
            // the height must be known when the list lays out the message
            asynchronous: false
            source: root.url
            autoTransform: true
        }
    }

    Component {
        id: animated
        AnimatedImage {
            width: root.maxWidth
            height: implicitWidth > 0 ? width * implicitHeight / implicitWidth : width * 0.75
            fillMode: Image.PreserveAspectFit
            source: root.url
        }
    }

    BackgroundItem {
        id: tile
        visible: !(root.ready && root.image)
        width: root.maxWidth
        height: Math.max(icon.height, labels.height) + 2 * Theme.paddingSmall

        Icon {
            id: icon
            x: Theme.paddingMedium
            anchors.verticalCenter: parent.verticalCenter
            source: !root.ready ? "image://theme/icon-m-cloud-download"
                  : root.video ? "image://theme/icon-m-play"
                  : "image://theme/icon-m-downloads"
        }

        Column {
            id: labels
            anchors {
                left: icon.right
                right: parent.right
                leftMargin: Theme.paddingMedium
                rightMargin: Theme.paddingMedium
                verticalCenter: parent.verticalCenter
            }

            Label {
                width: parent.width
                text: root.attachment.name
                truncationMode: TruncationMode.Fade
                font.pixelSize: Theme.fontSizeSmall
            }
            Label {
                width: parent.width
                text: Format.formatFileSize(root.attachment.size)
                      + (root.ready ? "" : "  " + (root.requested ? qsTr("Downloading...") : qsTr("Tap to download")))
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
            }
        }
    }

    MouseArea {
        anchors.fill: parent
        onClicked: root.open()
        onPressAndHold: root.pressAndHold()
    }
}
