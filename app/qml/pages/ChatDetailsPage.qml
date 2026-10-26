import QtQuick 2.5
import Sailfish.Silica 1.0
import Sailfish.Pickers 1.0

// A chat's people, photos and settings, and changes to a group.
Page {
    id: page

    property string chatId
    property var info: chatModel.chat(chatId)
    property var people: []
    property var media: []
    readonly property bool group: info.group === true
    readonly property bool canChange: group && info.left !== true

    function refresh() {
        info = chatModel.chat(chatId)
        people = daemon.participants(chatId)
        media = daemon.chatMedia(chatId)
    }

    function photos() {
        var found = []
        for (var i = 0; i < media.length; i++)
            if (media[i].mime.indexOf("image/") === 0)
                found.push(media[i])
        return found
    }

    function open(item) {
        if (item.mime.indexOf("video/") === 0) {
            pageStack.push(Qt.resolvedUrl("VideoPage.qml"), { path: item.path, mime: item.mime })
            return
        }
        var all = photos()
        for (var i = 0; i < all.length; i++)
            if (all[i].id === item.id)
                pageStack.push(Qt.resolvedUrl("GalleryPage.qml"), { photos: all, startIndex: i })
    }

    onStatusChanged: {
        if (status === PageStatus.Activating)
            refresh()
    }

    Connections {
        target: chatModel
        onModelReset: page.refresh()
    }

    Component {
        id: renameDialog
        Dialog {
            onAccepted: daemon.renameGroup(page.chatId, nameField.text)

            Column {
                width: parent.width

                DialogHeader { title: qsTr("Group name") }

                TextField {
                    id: nameField
                    width: parent.width
                    text: page.info.name || ""
                    label: qsTr("Empty shows the names of the people")
                    placeholderText: qsTr("Group name")
                    focus: true
                    EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                    EnterKey.onClicked: accept()
                }
            }
        }
    }

    Component {
        id: addDialog
        Dialog {
            canAccept: handleField.text.trim() !== ""
            onAccepted: daemon.addParticipant(page.chatId, handleField.text.trim())

            Column {
                width: parent.width

                DialogHeader { title: qsTr("Add person") }

                TextField {
                    id: handleField
                    width: parent.width
                    label: qsTr("Phone number or email")
                    placeholderText: label
                    inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText
                    focus: true
                    EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                    EnterKey.onClicked: accept()
                }
            }
        }
    }

    Component {
        id: photoPicker
        ImagePickerPage {
            onSelectedContentPropertiesChanged: {
                if (!daemon.setGroupPhoto(page.chatId, selectedContentProperties.filePath))
                    Notices.show(qsTr("Could not use this picture"), Notice.Short)
            }
        }
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        PullDownMenu {
            visible: page.canChange

            MenuItem {
                text: qsTr("Leave group")
                onClicked: Remorse.popupAction(page, qsTr("Leaving group"), function() { daemon.leaveGroup(page.chatId) })
            }
        }

        Column {
            id: column
            width: parent.width

            PageHeader {
                title: page.group ? qsTr("Group") : qsTr("Contact")
            }

            ListItem {
                id: photoItem
                contentHeight: bigAvatar.height + Theme.paddingLarge
                enabled: page.canChange
                onClicked: pageStack.push(photoPicker)
                menu: ContextMenu {
                    MenuItem {
                        text: qsTr("Change photo")
                        onClicked: pageStack.push(photoPicker)
                    }
                    MenuItem {
                        visible: (page.info.icon || "") !== ""
                        text: qsTr("Remove photo")
                        onClicked: daemon.setGroupPhoto(page.chatId, "")
                    }
                }

                Avatar {
                    id: bigAvatar
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: Theme.itemSizeHuge * 1.5
                    height: width
                    source: (page.info.icon || "") !== "" ? "file://" + page.info.icon
                          : !page.group && page.people.length > 0 ? contacts.avatars[page.people[0].handle] || ""
                          : ""
                    name: page.info.title || ""
                    group: page.group
                }

                Rectangle {
                    visible: page.canChange
                    anchors {
                        right: bigAvatar.right
                        bottom: bigAvatar.bottom
                        margins: bigAvatar.width * 0.05
                    }
                    width: Theme.iconSizeMedium
                    height: width
                    radius: width / 2
                    color: Theme.highlightBackgroundColor

                    Icon {
                        anchors.centerIn: parent
                        source: "image://theme/icon-s-edit"
                        color: Theme.primaryColor
                    }
                }
            }

            BackgroundItem {
                id: nameItem
                enabled: page.canChange
                height: nameLabel.height + 2 * Theme.paddingMedium
                onClicked: pageStack.push(renameDialog)

                Label {
                    id: nameLabel
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    anchors.verticalCenter: parent.verticalCenter
                    text: page.info.title || ""
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                    font.pixelSize: Theme.fontSizeLarge
                    color: nameItem.highlighted ? Theme.highlightColor : Theme.primaryColor
                }

                Rectangle {
                    visible: page.canChange
                    x: Theme.horizontalPageMargin
                    anchors.bottom: parent.bottom
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    height: 1
                    color: Theme.secondaryColor
                }
            }

            Item {
                width: parent.width
                height: Theme.paddingLarge
            }

            Row {
                anchors.horizontalCenter: parent.horizontalCenter
                height: Theme.itemSizeLarge

                DetailsButton {
                    width: column.width / 3
                    icon: page.info.muted === true ? "image://theme/icon-m-speaker-mute" : "image://theme/icon-m-speaker-on"
                    text: page.info.muted === true ? qsTr("Muted") : qsTr("Mute")
                    onClicked: daemon.setMuted(page.chatId, page.info.muted !== true)
                }

                DetailsButton {
                    visible: page.canChange
                    width: column.width / 3
                    icon: "image://theme/icon-m-add"
                    text: qsTr("Add person")
                    onClicked: pageStack.push(addDialog)
                }
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: page.info.muted === true
                text: qsTr("No notifications from this chat, on this phone only")
                wrapMode: Text.Wrap
                horizontalAlignment: Text.AlignHCenter
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
            }

            SectionHeader {
                visible: page.group
                text: qsTr("%1 members").arg(page.people.length + (page.info.left ? 0 : 1))
            }

            Repeater {
                model: page.people

                ListItem {
                    id: person
                    contentHeight: Theme.itemSizeMedium
                    menu: page.canChange ? personMenu : null

                    Component {
                        id: personMenu
                        ContextMenu {
                            MenuItem {
                                text: qsTr("Remove from group")
                                onClicked: {
                                    var handle = modelData.handle
                                    person.remorseAction(qsTr("Removing"), function() {
                                        daemon.removeParticipant(page.chatId, handle)
                                    })
                                }
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
                        width: person.contentHeight - 2 * Theme.paddingMedium
                        height: width
                        source: contacts.avatars[modelData.handle] || ""
                        name: modelData.name
                    }

                    Column {
                        anchors {
                            left: avatar.right
                            right: parent.right
                            leftMargin: Theme.paddingMedium
                            rightMargin: Theme.horizontalPageMargin
                            verticalCenter: parent.verticalCenter
                        }

                        Label {
                            width: parent.width
                            text: modelData.name
                            truncationMode: TruncationMode.Fade
                            color: person.highlighted ? Theme.highlightColor : Theme.primaryColor
                        }

                        Label {
                            width: parent.width
                            visible: text !== modelData.name
                            text: modelData.handle.replace(/^(tel|mailto):/, "")
                            truncationMode: TruncationMode.Fade
                            font.pixelSize: Theme.fontSizeSmall
                            color: Theme.secondaryColor
                        }
                    }
                }
            }

            SectionHeader {
                visible: page.media.length > 0
                text: qsTr("Photos and videos")
            }

            Grid {
                id: grid
                readonly property real cell: Math.floor((width - (columns - 1) * spacing) / columns)
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                columns: 4
                spacing: Theme.paddingSmall

                Repeater {
                    model: page.media

                    BackgroundItem {
                        width: grid.cell
                        height: grid.cell
                        onClicked: page.open(modelData)

                        Image {
                            anchors.fill: parent
                            visible: modelData.mime.indexOf("image/") === 0
                            source: visible ? "file://" + modelData.path : ""
                            sourceSize.width: width
                            sourceSize.height: height
                            fillMode: Image.PreserveAspectCrop
                            autoTransform: true
                            asynchronous: true
                            clip: true
                        }

                        Rectangle {
                            anchors.fill: parent
                            visible: modelData.mime.indexOf("video/") === 0
                            color: Theme.rgba(Theme.highlightBackgroundColor, Theme.opacityFaint)

                            Icon {
                                anchors.centerIn: parent
                                source: "image://theme/icon-m-play"
                            }
                        }
                    }
                }
            }
        }

        VerticalScrollDecorator {}
    }
}
