import QtQuick 2.0
import Sailfish.Silica 1.0
import Sailfish.Gallery 1.0

// A message's photos, swiped through like in the Gallery app.
FullscreenContentPage {
    id: page

    property var photos  // attachments: { path, mime, ... }
    property int startIndex
    property bool controls: true

    allowedOrientations: Orientation.All

    SlideshowView {
        id: view
        anchors.fill: parent
        model: page.photos
        currentIndex: page.startIndex

        delegate: ImageViewer {
            width: view.width
            height: view.height
            source: "file://" + modelData.path
            active: PathView.isCurrentItem
            onClicked: page.controls = !page.controls
        }
    }

    Label {
        visible: page.controls && page.photos.length > 1
        anchors {
            top: parent.top
            horizontalCenter: parent.horizontalCenter
            topMargin: Theme.paddingLarge
        }
        text: (view.currentIndex + 1) + " / " + page.photos.length
        color: Theme.highlightColor
    }

    SaveButton {
        visible: page.controls
        path: page.photos[view.currentIndex].path
        mime: page.photos[view.currentIndex].mime
    }
}
