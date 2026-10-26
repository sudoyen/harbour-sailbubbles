import QtQuick 2.0
import Sailfish.Silica 1.0
import Sailfish.Pickers 1.0
import harbour.sailbubbles 1.0
import "time.js" as Time
import "reactions.js" as Reactions

Page {
    id: page

    property string chatId
    property string title
    property var info: chatModel.chat(chatId)
    readonly property bool group: info.group === true
    readonly property bool leftGroup: info.left === true

    // set while replying to or editing a message
    property string replyTo
    property string replyText
    property string editing
    // what was typed before editing started, put back after
    property string draftText

    // a message to scroll to and briefly highlight, as from search
    property string focusMessage
    property bool focusDone
    property bool focusShown
    property int focusRow

    property bool otherTyping
    property bool sentTyping

    // Apple's limits for changing a sent message
    readonly property int editWindowMs: 15 * 60 * 1000
    readonly property int unsendWindowMs: 2 * 60 * 1000

    function reactionsText(reactions) {
        var parts = []
        for (var i = 0; i < reactions.length; i++)
            parts.push((reactions[i].who || qsTr("You")) + ": " + Reactions.label(reactions[i].kind))
        return parts.join(", ")
    }

    function ourReaction(reactions) {
        for (var i = 0; i < reactions.length; i++)
            if (reactions[i].who === "")
                return reactions[i].kind
        return ""
    }

    function eventText(kind, actor, text) {
        var who = actor === "" ? qsTr("You") : actor
        var whom = text === "" ? qsTr("you") : text
        switch (kind) {
        case "renamed": return text === "" ? qsTr("%1 removed the group name").arg(who)
                                           : qsTr("%1 named the group %2").arg(who).arg(text)
        case "added": return qsTr("%1 added %2").arg(who).arg(whom)
        case "removed": return qsTr("%1 removed %2").arg(who).arg(whom)
        case "left": return text === "" ? qsTr("You left the group") : qsTr("%1 left the group").arg(text)
        case "icon": return qsTr("%1 changed the group photo").arg(who)
        case "iconRemoved": return qsTr("%1 removed the group photo").arg(who)
        }
        return ""
    }

    function savable(attachments) {
        var found = []
        for (var i = 0; i < attachments.length; i++) {
            var a = attachments[i]
            if (a.path !== "" && (a.mime.indexOf("image/") === 0 || a.mime.indexOf("video/") === 0))
                found.push(a)
        }
        return found
    }

    function within(time, windowMs) {
        return Date.now() - time.getTime() < windowMs
    }

    function stopTyping() {
        typingTimer.stop()
        if (sentTyping) {
            sentTyping = false
            daemon.typing(chatId, false)
        }
    }

    function cancelCompose() {
        replyTo = ""
        replyText = ""
        if (editing !== "") {
            editing = ""
            input.text = draftText
        }
    }

    function finishCompose() {
        input.text = editing !== "" ? draftText : ""
        replyTo = ""
        replyText = ""
        editing = ""
        stopTyping()
        saveDraft()
    }

    function loadDraft() {
        var draft = daemon.draft(chatId)
        input.text = draft.text || ""
        var files = draft.files || []
        for (var i = 0; i < files.length; i++)
            picked.append(files[i])
    }

    function saveDraft() {
        var paths = []
        for (var i = 0; i < picked.count; i++)
            paths.push(picked.get(i).path)
        daemon.setDraft(chatId, editing !== "" ? draftText : input.text, paths)
    }

    function showFocusMessage() {
        focusDone = true
        var row = messages.loadUntil(focusMessage)
        if (row < 0)
            return
        list.positionViewAtIndex(row, ListView.Center)
        focusRow = row
        settleTimer.start()
        focusShown = true
        focusTimer.start()
    }

    function sendText() {
        // the keyboard holds back the word being typed until committed
        Qt.inputMethod.commit()
        var text = input.text
        var hasText = text.trim().length > 0
        if (editing !== "") {
            if (hasText && daemon.edit(editing, text))
                finishCompose()
            return
        }
        if (!hasText && picked.count === 0)
            return
        var to = [chatId]
        // As OpenBubbles does: each file goes as a message of its own, the
        // text after them, and the reply with the first.
        while (picked.count > 0) {
            var file = picked.get(0)
            if (daemon.sendFile(to, file.path, file.mime, replyTo) === "")
                return
            picked.remove(0)
            replyTo = ""
        }
        if (!hasText || daemon.send(to, text, replyTo) !== "")
            finishCompose()
    }

    onStatusChanged: {
        if (status === PageStatus.Active) {
            if (pageStack.nextPage() === null)
                pageStack.pushAttached(Qt.resolvedUrl("ChatDetailsPage.qml"), { chatId: chatId })
            daemon.markRead(chatId)
            if (focusMessage !== "" && !focusDone)
                showFocusMessage()
        } else if (status === PageStatus.Deactivating) {
            stopTyping()
            saveDraft()
        }
    }

    Component.onCompleted: loadDraft()

    // the app can be closed from the home screen while a chat is open
    Connections {
        target: Qt.application
        onStateChanged: {
            if (Qt.application.state !== Qt.ApplicationActive && page.status === PageStatus.Active)
                page.saveDraft()
        }
    }

    // delegates around the message only get their real heights once created,
    // which moves it; place it again after that
    Timer {
        id: settleTimer
        interval: 100
        onTriggered: list.positionViewAtIndex(page.focusRow, ListView.Center)
    }

    Timer {
        id: focusTimer
        interval: 2000
        onTriggered: page.focusShown = false
    }

    ListModel {
        id: picked
    }

    Component {
        id: picker
        MultiContentPickerDialog {
            title: qsTr("Attach")
            onAccepted: {
                for (var i = 0; i < selectedContent.count; i++) {
                    var c = selectedContent.get(i)
                    picked.append({ path: c.filePath, mime: c.mimeType, name: c.fileName })
                }
            }
        }
    }

    Connections {
        target: chatModel
        onModelReset: page.info = chatModel.chat(page.chatId)
    }

    MessageModel {
        id: messages
        service: daemon
        chatId: page.chatId
    }

    Connections {
        target: daemon
        onMessageReceived: {
            if (chatId === page.chatId && page.status === PageStatus.Active)
                daemon.markRead(page.chatId)
        }
        onTypingChanged: {
            if (chatId === page.chatId) {
                page.otherTyping = typing
                if (typing)
                    typingTimeout.restart()
            }
        }
    }

    // Apple devices stop the indicator after a while without news.
    Timer {
        id: typingTimeout
        interval: 60000
        onTriggered: page.otherTyping = false
    }

    // stop announcing typing after a pause
    Timer {
        id: typingTimer
        interval: 5000
        onTriggered: page.stopTyping()
    }

    PageHeader {
        id: header
        title: page.info.title || page.title

        MouseArea {
            anchors.fill: parent
            onClicked: pageStack.navigateForward()
        }
    }

    SilicaListView {
        id: list
        anchors {
            top: header.bottom
            left: parent.left
            right: parent.right
            bottom: composeArea.top
        }
        clip: true
        verticalLayoutDirection: ListView.BottomToTop
        spacing: Theme.paddingMedium
        model: messages

        onAtYBeginningChanged: {
            if (atYBeginning)
                messages.loadMore()
        }

        // bottom to top: the header sits under the newest message
        header: Item {
            width: list.width
            height: page.otherTyping ? typingLabel.height + Theme.paddingSmall : 0

            Label {
                id: typingLabel
                x: Theme.horizontalPageMargin
                visible: page.otherTyping
                text: qsTr("typing...")
                font.pixelSize: Theme.fontSizeExtraSmall
                font.italic: true
                color: Theme.secondaryHighlightColor
            }
        }

        delegate: ListItem {
            id: item
            width: list.width
            readonly property bool event: model.kind !== ""
            highlighted: down || menuOpen || (page.focusShown && model.messageId === page.focusMessage)
            readonly property real maxWidth: list.width * 0.75
            contentHeight: content.height
            menu: event ? null : messageMenu

            Component {
                id: messageMenu
                ContextMenu {
                    MenuItem {
                        visible: !model.unsent
                        text: qsTr("React")
                        onClicked: pageStack.push(Qt.resolvedUrl("ReactionPage.qml"),
                                                  { messageId: model.messageId, current: page.ourReaction(model.reactions) })
                    }
                    MenuItem {
                        visible: !model.unsent
                        text: qsTr("Reply")
                        onClicked: {
                            page.editing = ""
                            page.replyTo = model.messageId
                            page.replyText = model.text
                            input.forceActiveFocus()
                        }
                    }
                    MenuItem {
                        visible: page.savable(model.attachments).length > 0
                        text: qsTr("Save to Gallery")
                        onClicked: {
                            var files = page.savable(model.attachments)
                            var saved = 0
                            for (var i = 0; i < files.length; i++)
                                if (appService.saveToGallery(files[i].path, files[i].mime) !== "")
                                    saved++
                            Notices.show(saved === files.length ? qsTr("Saved to Gallery") : qsTr("Could not save"),
                                         Notice.Short)
                        }
                    }
                    MenuItem {
                        visible: model.edited && !model.unsent
                        text: qsTr("Edit history")
                        onClicked: pageStack.push(Qt.resolvedUrl("EditHistoryPage.qml"),
                                                  { messageId: model.messageId, current: model.text })
                    }
                    MenuItem {
                        visible: !model.unsent && model.text !== ""
                        text: qsTr("Copy")
                        onClicked: Clipboard.text = model.text
                    }
                    MenuItem {
                        visible: model.mine && !model.unsent && page.within(model.time, page.editWindowMs)
                        text: qsTr("Edit")
                        onClicked: {
                            page.replyTo = ""
                            if (page.editing === "")
                                page.draftText = input.text
                            page.editing = model.messageId
                            input.text = model.text
                            input.forceActiveFocus()
                        }
                    }
                    MenuItem {
                        visible: model.mine && !model.unsent && page.within(model.time, page.unsendWindowMs)
                        text: qsTr("Unsend")
                        onClicked: {
                            var id = model.messageId
                            item.remorseAction(qsTr("Unsending"), function() { daemon.unsend(id) })
                        }
                    }
                    MenuItem {
                        text: qsTr("Delete")
                        onClicked: {
                            var id = model.messageId
                            item.remorseDelete(function() { daemon.deleteMessage(id) })
                        }
                    }
                }
            }

            Column {
                id: content
                width: parent.width

                Label {
                    visible: model.newDay
                    width: parent.width
                    height: visible ? implicitHeight + 2 * Theme.paddingMedium : 0
                    text: Time.heading(model.time)
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: Theme.secondaryColor
                }

                Label {
                    visible: item.event
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    text: item.event ? page.eventText(model.kind, model.senderName, model.text) + "  " + Time.clock(model.time)
                                     : ""
                    wrapMode: Text.Wrap
                    horizontalAlignment: Text.AlignHCenter
                    font.pixelSize: Theme.fontSizeExtraSmall
                    font.italic: true
                    color: Theme.secondaryColor
                }

                Label {
                    x: Theme.horizontalPageMargin
                    visible: !item.event && page.group && !model.mine && model.firstOfRun
                    text: model.senderName
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: Theme.highlightColor
                }

                Rectangle {
                    id: bubble
                    visible: !item.event
                    x: model.mine ? parent.width - width - Theme.horizontalPageMargin : Theme.horizontalPageMargin
                    width: inner.width + 2 * inner.x
                    height: inner.height + 2 * Theme.paddingSmall
                    radius: Theme.paddingMedium
                    color: model.unsent ? "transparent"
                         : model.mine ? Theme.rgba(Theme.highlightBackgroundColor, 0.4)
                         : Theme.rgba(Theme.primaryColor, 0.1)
                    border.width: model.unsent ? 1 : 0
                    border.color: Theme.secondaryColor

                    Column {
                        id: inner
                        x: files.visible ? Theme.paddingSmall : Theme.paddingMedium
                        y: Theme.paddingSmall
                        spacing: Theme.paddingSmall

                        Item {
                            visible: model.replyTo !== ""
                            width: replyLabel.x + replyLabel.width
                            height: replyLabel.height

                            Rectangle {
                                width: 2
                                height: parent.height
                                color: Theme.highlightColor
                            }

                            Label {
                                id: replyLabel
                                x: Theme.paddingSmall
                                width: Math.min(implicitWidth, item.maxWidth - x)
                                text: model.replyText || qsTr("an earlier message")
                                truncationMode: TruncationMode.Fade
                                maximumLineCount: 1
                                font.pixelSize: Theme.fontSizeExtraSmall
                                color: Theme.secondaryColor
                            }
                        }

                        AttachmentGroup {
                            id: files
                            visible: attachments.length > 0
                            attachments: model.unsent ? [] : model.attachments
                            maxWidth: item.maxWidth
                            onPressAndHold: item.openMenu()
                        }

                        Label {
                            visible: model.unsent || model.text !== ""
                            width: Math.min(implicitWidth, item.maxWidth)
                            text: model.unsent ? (model.mine ? qsTr("You unsent a message") : qsTr("This message was unsent"))
                                               : model.text
                            wrapMode: Text.Wrap
                            font.italic: model.unsent
                            color: model.unsent ? Theme.secondaryColor
                                 : item.highlighted ? Theme.highlightColor : Theme.primaryColor
                        }

                        Row {
                            x: inner.width - width
                            spacing: Theme.paddingSmall

                            Label {
                                visible: model.edited
                                text: qsTr("Edited")
                                font.pixelSize: Theme.fontSizeTiny
                                color: Theme.secondaryColor
                            }

                            Label {
                                id: timeLabel
                                text: model.mine && model.status === Daemon.Failed ? qsTr("Not sent") : Time.clock(model.time)
                                font.pixelSize: Theme.fontSizeTiny
                                color: model.mine && model.status === Daemon.Failed ? Theme.errorColor : Theme.secondaryColor
                            }

                            StatusTicks {
                                visible: model.mine && !model.unsent
                                anchors.verticalCenter: timeLabel.verticalCenter
                                size: timeLabel.height
                                status: model.status
                            }
                        }
                    }
                }

                Rectangle {
                    visible: !item.event && model.reactions.length > 0
                    x: model.mine ? bubble.x + bubble.width - width : bubble.x
                    width: reactionsLabel.width + 2 * Theme.paddingMedium
                    height: reactionsLabel.height + Theme.paddingSmall
                    radius: height / 2
                    color: Theme.rgba(Theme.highlightBackgroundColor, 0.25)

                    Label {
                        id: reactionsLabel
                        anchors.centerIn: parent
                        width: Math.min(implicitWidth, item.maxWidth)
                        text: page.reactionsText(model.reactions)
                        wrapMode: Text.Wrap
                        font.pixelSize: Theme.fontSizeExtraSmall
                        color: Theme.highlightColor
                    }
                }
            }
        }

        VerticalScrollDecorator {}
    }

    Column {
        id: composeArea
        anchors {
            left: parent.left
            right: parent.right
            bottom: parent.bottom
        }

        BackgroundItem {
            width: parent.width
            height: visible ? Theme.itemSizeExtraSmall : 0
            visible: page.replyTo !== "" || page.editing !== ""
            onClicked: page.cancelCompose()

            Label {
                anchors {
                    left: parent.left
                    right: cancelIcon.left
                    leftMargin: Theme.horizontalPageMargin
                    rightMargin: Theme.paddingMedium
                    verticalCenter: parent.verticalCenter
                }
                text: page.editing !== "" ? qsTr("Editing message")
                                          : qsTr("Replying to: %1").arg(page.replyText)
                truncationMode: TruncationMode.Fade
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.highlightColor
            }

            Icon {
                id: cancelIcon
                anchors {
                    right: parent.right
                    rightMargin: Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                }
                source: "image://theme/icon-m-clear"
            }
        }

        PickedFiles {
            width: parent.width
            model: picked
        }

        Label {
            visible: page.leftGroup
            x: Theme.horizontalPageMargin
            width: parent.width - 2 * Theme.horizontalPageMargin
            height: visible ? implicitHeight + 2 * Theme.paddingLarge : 0
            verticalAlignment: Text.AlignVCenter
            text: qsTr("You are no longer in this group")
            wrapMode: Text.Wrap
            color: Theme.secondaryHighlightColor
        }

        Item {
            visible: !page.leftGroup
            width: parent.width
            height: visible ? input.height : 0

            TextArea {
                id: input
                anchors {
                    left: parent.left
                    right: sendButton.left
                }
                placeholderText: qsTr("Message")
                leftItem: IconButton {
                    visible: page.editing === ""
                    width: visible ? icon.width : 0
                    icon.source: "image://theme/icon-m-attach"
                    enabled: daemon.available
                    onClicked: pageStack.push(picker)
                }
                label: daemon.available ? "" : qsTr("Message service is not running")
                onTextChanged: {
                    if (page.editing !== "" || text.length === 0) {
                        page.stopTyping()
                        return
                    }
                    if (!page.sentTyping) {
                        page.sentTyping = true
                        daemon.typing(page.chatId, true)
                    }
                    typingTimer.restart()
                }
            }

            IconButton {
                id: sendButton
                anchors {
                    right: parent.right
                    rightMargin: Theme.horizontalPageMargin
                    verticalCenter: input.top
                    verticalCenterOffset: input.textVerticalCenterOffset
                }
                icon.source: page.editing !== "" ? "image://theme/icon-m-acknowledge" : "image://theme/icon-m-send"
                enabled: daemon.available && (input.text.trim().length > 0 || (page.editing === "" && picked.count > 0))
                onClicked: page.sendText()
            }
        }
    }
}
