import QtQuick 2.0
import Sailfish.Silica 1.0

// Chats kept out of the main list: archived ones, or those with unknown senders.
Page {
    id: page

    property alias chats: list.model
    property string title
    property string emptyText

    SilicaListView {
        id: list
        anchors.fill: parent

        header: PageHeader { title: page.title }

        ViewPlaceholder {
            enabled: list.count === 0
            text: page.emptyText
        }

        delegate: ChatItem {}

        VerticalScrollDecorator {}
    }
}
