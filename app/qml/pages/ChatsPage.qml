import QtQuick 2.0
import Sailfish.Silica 1.0

Page {
    id: page

    SilicaListView {
        id: list
        anchors.fill: parent
        model: chatModel

        header: PageHeader { title: qsTr("Messages") }

        PullDownMenu {
            MenuItem {
                visible: unknownChats.count > 0
                text: qsTr("Unknown senders (%1)").arg(unknownChats.count)
                onClicked: pageStack.push(Qt.resolvedUrl("ChatListPage.qml"),
                                          { chats: unknownChats, title: qsTr("Unknown senders"),
                                            emptyText: qsTr("No chats with unknown senders") })
            }
            MenuItem {
                visible: archivedChats.count > 0
                text: qsTr("Archived (%1)").arg(archivedChats.count)
                onClicked: pageStack.push(Qt.resolvedUrl("ChatListPage.qml"),
                                          { chats: archivedChats, title: qsTr("Archived"),
                                            emptyText: qsTr("No archived chats") })
            }
            MenuItem {
                text: qsTr("Settings")
                enabled: daemon.available
                onClicked: pageStack.push(Qt.resolvedUrl("SettingsPage.qml"))
            }
            MenuItem {
                text: qsTr("Search")
                enabled: daemon.available
                onClicked: pageStack.push(Qt.resolvedUrl("SearchPage.qml"))
            }
            MenuItem {
                text: qsTr("New message")
                enabled: daemon.available
                onClicked: pageStack.push(Qt.resolvedUrl("NewMessagePage.qml"))
            }
        }

        ViewPlaceholder {
            enabled: list.count === 0
            text: daemon.available ? qsTr("No conversations yet") : qsTr("Message service is not running")
            hintText: daemon.available ? qsTr("Pull down to start one")
                                       : qsTr("Start it with systemctl --user start sailbubbles-daemon")
        }

        delegate: ChatItem {}

        VerticalScrollDecorator {}
    }
}
