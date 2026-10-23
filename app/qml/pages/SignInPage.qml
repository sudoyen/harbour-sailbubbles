import QtQuick 2.0
import Sailfish.Silica 1.0

// Signing in to Apple, one step at a time as the daemon asks.
Page {
    id: page
    objectName: "signIn"

    readonly property string step: daemon.signInStep
    property string problem

    function answer(values) {
        problem = daemon.answerSignIn(values)
    }

    // "id:last two digits,..." from the daemon
    function phones() {
        var found = []
        var parts = daemon.signInDetail.split(",")
        for (var i = 0; i < parts.length; i++) {
            var p = parts[i].split(":")
            if (p.length === 2)
                found.push({ id: p[0], last: p[1] })
        }
        return found
    }

    onStepChanged: {
        problem = ""
        code.text = ""
        password.text = ""
        relayCode.text = ""
    }

    backNavigation: false

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingMedium

            PageHeader { title: qsTr("Sign in to iMessage") }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                text: {
                    switch (page.step) {
                    case "relay": return qsTr("SailBubbles signs in through a Mac running the registration relay. Enter the relay's address and the code it shows.")
                    case "account": return qsTr("Your Apple ID and password. They go to Apple only and are not stored.")
                    case "deviceCode": return qsTr("Apple sent a code to your iPhone, iPad or Mac.")
                    case "phone": return qsTr("Send a code by SMS to:")
                    case "smsCode": return qsTr("Enter the code from the SMS.")
                    case "failed": return qsTr("Signing in did not work:")
                    }
                    return daemon.signInDetail
                }
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: page.step === "failed"
                wrapMode: Text.Wrap
                text: daemon.signInDetail
            }

            BusyIndicator {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: page.step === "working"
                running: visible
                size: BusyIndicatorSize.Large
            }

            TextField {
                id: relayHost
                visible: page.step === "relay"
                width: parent.width
                label: qsTr("Relay address, on your own network")
                placeholderText: "http://192.168.1.23:8000"
                text: page.step === "relay" ? daemon.signInDetail : ""
                inputMethodHints: Qt.ImhUrlCharactersOnly | Qt.ImhNoPredictiveText
                EnterKey.iconSource: "image://theme/icon-m-enter-next"
                EnterKey.onClicked: relayCode.focus = true
            }

            PasswordField {
                id: relayCode
                visible: page.step === "relay"
                width: parent.width
                label: daemon.signInDetail !== "" ? qsTr("Relay code, empty to keep the old one") : qsTr("Relay code")
                placeholderText: qsTr("Relay code")
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: page.answer([relayHost.text, relayCode.text])
            }

            TextField {
                id: appleId
                visible: page.step === "account"
                width: parent.width
                label: qsTr("Apple ID")
                placeholderText: label
                inputMethodHints: Qt.ImhEmailCharactersOnly | Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText
                EnterKey.iconSource: "image://theme/icon-m-enter-next"
                EnterKey.onClicked: password.focus = true
            }

            PasswordField {
                id: password
                visible: page.step === "account"
                width: parent.width
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: page.answer([appleId.text, password.text])
            }

            TextField {
                id: code
                visible: page.step === "deviceCode" || page.step === "smsCode"
                width: parent.width
                label: qsTr("Verification code")
                placeholderText: "000000"
                inputMethodHints: Qt.ImhDigitsOnly
                validator: RegExpValidator { regExp: /^[0-9]{0,6}$/ }
                EnterKey.enabled: text.length === 6
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: page.answer([text])
            }

            Repeater {
                model: page.step === "phone" ? page.phones() : []

                BackgroundItem {
                    width: column.width
                    onClicked: page.answer([modelData.id])

                    Label {
                        x: Theme.horizontalPageMargin
                        anchors.verticalCenter: parent.verticalCenter
                        text: qsTr("Number ending in %1").arg(modelData.last)
                        color: parent.highlighted ? Theme.highlightColor : Theme.primaryColor
                    }
                }
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: page.problem !== ""
                wrapMode: Text.Wrap
                text: page.problem
                color: Theme.errorColor
                font.pixelSize: Theme.fontSizeSmall
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: page.step !== "working" && page.step !== "phone" && page.step !== ""
                text: page.step === "failed" ? qsTr("Try again") : qsTr("Continue")
                enabled: {
                    switch (page.step) {
                    case "relay": return relayHost.text.trim() !== ""
                    case "account": return appleId.text.trim() !== "" && password.text !== ""
                    case "deviceCode":
                    case "smsCode": return code.text.length === 6
                    }
                    return true
                }
                onClicked: {
                    switch (page.step) {
                    case "relay": page.answer([relayHost.text, relayCode.text]); break
                    case "account": page.answer([appleId.text, password.text]); break
                    case "deviceCode":
                    case "smsCode": page.answer([code.text]); break
                    case "failed": page.answer([]); break
                    }
                }
            }
        }

        VerticalScrollDecorator {}
    }
}
