import QtQuick 2.0
import Sailfish.Silica 1.0
import "reactions.js" as Reactions

Page {
    id: page

    property string messageId
    // our current reaction kind, or ""
    property string current

    SilicaListView {
        anchors.fill: parent
        header: PageHeader { title: qsTr("React") }
        model: Reactions.kinds.concat(page.current !== "" ? [""] : [])

        delegate: BackgroundItem {
            highlighted: down || modelData === page.current

            Label {
                x: Theme.horizontalPageMargin
                anchors.verticalCenter: parent.verticalCenter
                text: modelData === "" ? qsTr("Remove reaction") : Reactions.label(modelData)
                color: highlighted ? Theme.highlightColor : Theme.primaryColor
            }

            onClicked: {
                daemon.react(page.messageId, modelData === page.current ? "" : modelData)
                pageStack.pop()
            }
        }

        VerticalScrollDecorator {}
    }
}
