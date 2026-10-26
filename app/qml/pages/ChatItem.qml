import QtQuick 2.0
import Sailfish.Silica 1.0
import "time.js" as Time

// One chat in a list of chats.
ListItem {
    id: item
    contentHeight: Theme.itemSizeMedium

    // a group shows its own photo or none, never a member's
    function avatarOf(participants, icon, group) {
        if (icon !== "")
            return "file://" + icon
        var avatars = contacts.avatars
        if (group)
            return ""
        for (var i = 0; i < participants.length; i++)
            if (avatars[participants[i]])
                return avatars[participants[i]]
        return ""
    }

    onClicked: pageStack.push(Qt.resolvedUrl("ConversationPage.qml"),
                              { chatId: model.chatId, title: model.title })

    menu: ContextMenu {
        MenuItem {
            text: model.pinned ? qsTr("Unpin") : qsTr("Pin")
            onClicked: daemon.setPinned(model.chatId, !model.pinned)
        }
        MenuItem {
            text: model.muted ? qsTr("Unmute") : qsTr("Mute")
            onClicked: daemon.setMuted(model.chatId, !model.muted)
        }
        MenuItem {
            text: model.unread > 0 ? qsTr("Mark as read") : qsTr("Mark as unread")
            onClicked: model.unread > 0 ? daemon.markRead(model.chatId) : daemon.markUnread(model.chatId)
        }
        MenuItem {
            text: model.archived ? qsTr("Unarchive") : qsTr("Archive")
            onClicked: daemon.setArchived(model.chatId, !model.archived)
        }
        MenuItem {
            text: qsTr("Delete")
            onClicked: {
                var id = model.chatId
                item.remorseDelete(function() { daemon.deleteChat(id) })
            }
        }
    }

    Avatar {
        id: avatar
        anchors {
            left: parent.left
            leftMargin: Theme.horizontalPageMargin
            verticalCenter: parent.verticalCenter
        }
        width: item.contentHeight - 2 * Theme.paddingSmall
        height: width
        source: item.avatarOf(model.participants, model.icon, model.group)
        name: model.title
        group: model.group
    }

    Icon {
        id: mutedIcon
        anchors {
            right: pinIcon.left
            rightMargin: Theme.paddingSmall
            verticalCenter: timeLabel.verticalCenter
        }
        visible: model.muted
        width: visible ? Theme.iconSizeSmall : 0
        height: Theme.iconSizeSmall
        sourceSize.width: Theme.iconSizeSmall
        sourceSize.height: Theme.iconSizeSmall
        source: "image://theme/icon-m-silent"
    }

    Icon {
        id: pinIcon
        anchors {
            right: timeLabel.left
            rightMargin: Theme.paddingSmall
            verticalCenter: timeLabel.verticalCenter
        }
        visible: model.pinned
        width: visible ? implicitWidth : 0
        source: "image://theme/icon-s-favorite"
    }

    Label {
        id: titleLabel
        anchors {
            left: avatar.right
            right: mutedIcon.left
            leftMargin: Theme.paddingMedium
            rightMargin: Theme.paddingMedium
            top: parent.top
            topMargin: Theme.paddingSmall
        }
        text: model.title
        truncationMode: TruncationMode.Fade
        color: model.unread > 0 || item.highlighted ? Theme.highlightColor : Theme.primaryColor
    }

    Label {
        id: timeLabel
        anchors {
            right: parent.right
            rightMargin: Theme.horizontalPageMargin
            baseline: titleLabel.baseline
        }
        text: Time.short(model.lastTime)
        font.pixelSize: Theme.fontSizeExtraSmall
        color: item.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
    }

    Label {
        anchors {
            left: titleLabel.left
            right: unreadBadge.visible ? unreadBadge.left : timeLabel.right
            rightMargin: unreadBadge.visible ? Theme.paddingMedium : 0
            top: titleLabel.bottom
        }
        text: model.lastText
        truncationMode: TruncationMode.Fade
        maximumLineCount: 1
        font.pixelSize: Theme.fontSizeSmall
        color: item.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
    }

    Rectangle {
        id: unreadBadge
        visible: model.unread > 0
        anchors {
            right: parent.right
            rightMargin: Theme.horizontalPageMargin
            top: timeLabel.bottom
            topMargin: Theme.paddingSmall
        }
        width: Math.max(height, unreadCount.implicitWidth + Theme.paddingMedium)
        height: unreadCount.implicitHeight
        radius: height / 2
        color: Theme.highlightBackgroundColor

        Label {
            id: unreadCount
            anchors.centerIn: parent
            text: model.unread
            font.pixelSize: Theme.fontSizeExtraSmall
            color: Theme.primaryColor
        }
    }
}
