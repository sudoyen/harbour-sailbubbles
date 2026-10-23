import QtQuick 2.0
import Sailfish.Silica 1.0

Page {
    id: page

    property string error

    // several recipients make a group chat
    function recipients() {
        var to = []
        var parts = recipient.text.split(",")
        for (var i = 0; i < parts.length; i++)
            if (parts[i].trim() !== "")
                to.push(parts[i].trim())
        return to
    }

    function sendText() {
        var to = recipients()
        var chat = daemon.send(to, body.text)
        if (chat !== "")
            pageStack.replace(Qt.resolvedUrl("ConversationPage.qml"), { chatId: chat, title: to.join(", ") })
    }

    Connections {
        target: daemon
        onSendFailed: page.error = error
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height

        Column {
            id: column
            width: parent.width

            PageHeader { title: qsTr("New message") }

            TextField {
                id: recipient
                width: parent.width
                label: qsTr("Phone numbers or emails, separated by commas")
                placeholderText: qsTr("Phone number or email")
                inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText
                EnterKey.iconSource: "image://theme/icon-m-enter-next"
                EnterKey.onClicked: body.focus = true
                onTextChanged: page.error = ""
            }

            TextArea {
                id: body
                width: parent.width
                label: qsTr("Message")
                placeholderText: label
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: page.error !== ""
                text: page.error
                wrapMode: Text.Wrap
                color: Theme.errorColor
                font.pixelSize: Theme.fontSizeSmall
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                text: qsTr("Send")
                enabled: daemon.available && page.recipients().length > 0 && body.text.trim().length > 0
                onClicked: sendText()
            }
        }

        VerticalScrollDecorator {}
    }
}
