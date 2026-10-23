import QtQuick 2.0
import Sailfish.Silica 1.0
import "time.js" as Time

// The texts an edited message had before, oldest first, then its text now.
Page {
    id: page

    property string messageId
    property string current
    property var versions: daemon.editHistory(messageId)

    SilicaListView {
        id: list
        anchors.fill: parent
        model: page.versions.concat([{ text: page.current, time: null }])
        spacing: Theme.paddingLarge

        header: PageHeader { title: qsTr("Edit history") }

        delegate: Column {
            x: Theme.horizontalPageMargin
            width: list.width - 2 * Theme.horizontalPageMargin

            Label {
                width: parent.width
                text: modelData.time ? Time.full(modelData.time) : qsTr("Now")
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryHighlightColor
            }

            Label {
                width: parent.width
                text: modelData.text
                wrapMode: Text.Wrap
                color: modelData.time ? Theme.secondaryColor : Theme.primaryColor
            }
        }

        VerticalScrollDecorator {}
    }
}
