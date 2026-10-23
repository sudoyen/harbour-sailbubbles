import QtQuick 2.5
import Sailfish.Silica 1.0

// A message's attachments. Two or more photos form a grid (as many apps do);
// tapping one opens them all in a swipeable viewer.
Column {
    id: group

    property var attachments
    property real maxWidth

    readonly property var photos: attachments.filter(function(a) {
        return a.path !== "" && a.mime.indexOf("image/") === 0 && a.mime !== "image/gif"
    })
    readonly property bool grid: photos.length > 1
    readonly property int shown: Math.min(photos.length, 4)

    signal pressAndHold

    function openPhoto(index) {
        pageStack.push(Qt.resolvedUrl("GalleryPage.qml"), { photos: photos, startIndex: index })
    }

    spacing: Theme.paddingSmall

    Grid {
        id: photoGrid
        visible: group.grid
        columns: 2
        spacing: Theme.paddingSmall

        readonly property real cell: (group.maxWidth - spacing) / 2

        Repeater {
            model: group.grid ? group.photos.slice(0, group.shown) : []

            Item {
                width: photoGrid.cell
                height: photoGrid.cell

                Image {
                    anchors.fill: parent
                    source: "file://" + modelData.path
                    autoTransform: true
                    sourceSize.width: photoGrid.cell
                    sourceSize.height: photoGrid.cell
                    fillMode: Image.PreserveAspectCrop
                    clip: true
                }

                Rectangle {
                    anchors.fill: parent
                    visible: index === group.shown - 1 && group.photos.length > group.shown
                    color: Theme.rgba("black", 0.5)

                    Label {
                        anchors.centerIn: parent
                        text: "+" + (group.photos.length - group.shown + 1)
                        font.pixelSize: Theme.fontSizeExtraLarge
                    }
                }

                MouseArea {
                    anchors.fill: parent
                    onClicked: group.openPhoto(index)
                    onPressAndHold: group.pressAndHold()
                }
            }
        }
    }

    Repeater {
        model: group.attachments.filter(function(a) { return !group.grid || group.photos.indexOf(a) < 0 })

        AttachmentItem {
            attachment: modelData
            maxWidth: group.maxWidth
            onOpenPhoto: group.openPhoto(group.photos.indexOf(modelData))
            onPressAndHold: group.pressAndHold()
        }
    }
}
