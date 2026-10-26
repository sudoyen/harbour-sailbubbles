import QtQuick 2.0
import Sailfish.Silica 1.0

// An icon with its name under it, for the row of actions on the details page.
BackgroundItem {
    id: button

    property alias icon: image.source
    property alias text: label.text

    height: parent.height

    Icon {
        id: image
        anchors {
            horizontalCenter: parent.horizontalCenter
            bottom: parent.verticalCenter
        }
        highlighted: button.highlighted
    }

    Label {
        id: label
        anchors {
            horizontalCenter: parent.horizontalCenter
            top: image.bottom
        }
        width: parent.width - 2 * Theme.paddingSmall
        horizontalAlignment: Text.AlignHCenter
        truncationMode: TruncationMode.Fade
        font.pixelSize: Theme.fontSizeExtraSmall
        color: button.highlighted ? Theme.highlightColor : Theme.primaryColor
    }
}
