import QtQuick 2.0
import Sailfish.Silica 1.0
import "time.js" as Time

Page {
    id: page

    property string term
    property var results: []

    // the line fades out at its end, so start shortly before the match
    function snippet(text) {
        var at = text.toLowerCase().indexOf(term.trim().toLowerCase())
        return at > 20 ? "..." + text.substr(at - 15) : text
    }

    // the daemon walks every message, so not on each key press
    Timer {
        id: searchTimer
        interval: 300
        onTriggered: page.results = daemon.search(page.term)
    }

    // outside the list: new results reset it, which took the keyboard away
    Column {
        id: top
        width: parent.width

        PageHeader { title: qsTr("Search") }

        SearchField {
            width: parent.width
            placeholderText: qsTr("Search messages")
            focus: true
            onTextChanged: {
                page.term = text
                searchTimer.restart()
            }
            EnterKey.iconSource: "image://theme/icon-m-enter-close"
            EnterKey.onClicked: focus = false
        }
    }

    SilicaListView {
        id: list
        anchors {
            top: top.bottom
            left: parent.left
            right: parent.right
            bottom: parent.bottom
        }
        clip: true
        model: page.results

        ViewPlaceholder {
            enabled: list.count === 0 && page.term.trim() !== "" && !searchTimer.running
            text: qsTr("Nothing found")
        }

        delegate: ListItem {
            contentHeight: Theme.itemSizeMedium

            onClicked: pageStack.push(Qt.resolvedUrl("ConversationPage.qml"),
                                      { chatId: modelData.chatId, title: modelData.title,
                                        focusMessage: modelData.messageId })

            Label {
                id: titleLabel
                anchors {
                    left: parent.left
                    right: timeLabel.left
                    leftMargin: Theme.horizontalPageMargin
                    rightMargin: Theme.paddingMedium
                    top: parent.top
                    topMargin: Theme.paddingSmall
                }
                text: modelData.messageId === "" ? Theme.highlightText(modelData.title, page.term.trim(), Theme.highlightColor)
                                                 : modelData.title
                textFormat: modelData.messageId === "" ? Text.StyledText : Text.PlainText
                truncationMode: TruncationMode.Fade
            }

            Label {
                id: timeLabel
                anchors {
                    right: parent.right
                    rightMargin: Theme.horizontalPageMargin
                    baseline: titleLabel.baseline
                }
                text: Time.short(modelData.time)
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
            }

            Label {
                anchors {
                    left: titleLabel.left
                    right: timeLabel.right
                    top: titleLabel.bottom
                }
                text: modelData.messageId !== "" ? Theme.highlightText(page.snippet(modelData.text), page.term.trim(), Theme.highlightColor)
                                                 : modelData.text
                textFormat: modelData.messageId !== "" ? Text.StyledText : Text.PlainText
                truncationMode: TruncationMode.Fade
                maximumLineCount: 1
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryColor
            }
        }

        VerticalScrollDecorator {}
    }
}
