import QtQuick 2.0
import Sailfish.Silica 1.0

IconButton {
    property string path
    property string mime

    anchors {
        right: parent.right
        bottom: parent.bottom
        margins: Theme.horizontalPageMargin
    }
    icon.source: "image://theme/icon-m-device-download"
    onClicked: Notices.show(appService.saveToGallery(path, mime) !== ""
                            ? qsTr("Saved to Gallery") : qsTr("Could not save"), Notice.Short)
}
