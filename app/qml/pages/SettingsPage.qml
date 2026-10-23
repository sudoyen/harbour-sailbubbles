import QtQuick 2.0
import Sailfish.Silica 1.0
import "time.js" as Time

Page {
    id: page

    property var settings: daemon.settings()
    property var status: daemon.status()

    function pushText(state) {
        switch (state) {
        case "connected": return qsTr("Connected")
        case "connecting": return qsTr("Connecting")
        case "offline": return qsTr("Offline, retrying")
        }
        return qsTr("Unknown")
    }

    function registrationText(state) {
        switch (state) {
        case "ok": return qsTr("Registered")
        case "renewing": return qsTr("Renewing")
        case "retrying": return qsTr("Renewal failed, retrying")
        case "signin": return qsTr("Sign in again")
        case "stopped": return qsTr("Stopped")
        }
        return qsTr("Unknown")
    }

    Connections {
        target: daemon
        onStatusChanged: status = daemon.status()
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height

        Column {
            id: column
            width: parent.width

            PageHeader { title: qsTr("Settings") }

            SectionHeader { text: qsTr("Connection") }

            DetailItem {
                label: qsTr("Apple push")
                value: daemon.available ? pushText(status.push) : qsTr("Message service is not running")
            }

            DetailItem {
                label: qsTr("iMessage registration")
                value: registrationText(status.registration)
            }

            DetailItem {
                visible: status.renewAt !== undefined && !isNaN(status.renewAt.getTime())
                label: qsTr("Next renewal")
                value: visible ? Time.full(status.renewAt) : ""
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                text: status.registration === "signin"
                      ? qsTr("Apple wants a new sign-in.")
                      : qsTr("Renewing the registration needs your Mac relay to be reachable.")
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: status.registration === "signin" ? Theme.errorColor : Theme.secondaryColor
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                text: qsTr("Sign in again")
                enabled: daemon.available
                onClicked: Remorse.popupAction(page, qsTr("Signing out of iMessage"), function() { daemon.signInAgain() })
            }

            SectionHeader { text: qsTr("Notifications") }

            TextSwitch {
                text: qsTr("Show message text")
                description: qsTr("The text also appears on the lock screen. When off, notifications only say who wrote.")
                checked: settings.previews === true
                onClicked: {
                    if (!daemon.setSetting("previews", checked))
                        checked = !checked
                }
            }

            SectionHeader { text: qsTr("Chat list") }

            TextSwitch {
                text: qsTr("Filter unknown senders")
                description: qsTr("Chats with people who are not in your contacts get no notifications and are listed apart.")
                checked: settings.filterUnknown === true
                onClicked: {
                    if (!daemon.setSetting("filterUnknown", checked))
                        checked = !checked
                }
            }

            TextSwitch {
                text: qsTr("Unarchive on new message")
                description: qsTr("An archived chat comes back to the list when someone writes in it.")
                checked: settings.unarchiveOnNewMessage === true
                onClicked: {
                    if (!daemon.setSetting("unarchiveOnNewMessage", checked))
                        checked = !checked
                }
            }

            SectionHeader { text: qsTr("Messages") }

            TextSwitch {
                text: qsTr("Send read receipts")
                description: qsTr("People see when you have read their messages. Group chats never get read receipts.")
                checked: settings.readReceipts === true
                onClicked: {
                    if (!daemon.setSetting("readReceipts", checked))
                        checked = !checked
                }
            }
        }

        VerticalScrollDecorator {}
    }
}
