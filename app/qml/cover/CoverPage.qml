import QtQuick 2.0
import Sailfish.Silica 1.0

CoverBackground {
    Column {
        anchors.centerIn: parent
        width: parent.width - 2 * Theme.paddingLarge
        spacing: Theme.paddingSmall

        Label {
            anchors.horizontalCenter: parent.horizontalCenter
            text: chatModel.totalUnread
            font.pixelSize: Theme.fontSizeHuge
        }

        Label {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            text: qsTr("unread", "", chatModel.totalUnread)
            color: Theme.secondaryColor
            wrapMode: Text.Wrap
        }
    }

    CoverActionList {
        enabled: daemon.available

        CoverAction {
            iconSource: "image://theme/icon-cover-new"
            onTriggered: {
                appWindow.activate()
                pageStack.push(Qt.resolvedUrl("../pages/NewMessagePage.qml"), {}, PageStackAction.Immediate)
            }
        }
    }
}
