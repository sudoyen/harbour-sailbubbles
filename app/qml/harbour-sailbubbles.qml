import QtQuick 2.0
import Sailfish.Silica 1.0
import "pages"

ApplicationWindow {
    id: appWindow
    initialPage: Component { ChatsPage {} }
    cover: Qt.resolvedUrl("cover/CoverPage.qml")
    allowedOrientations: defaultAllowedOrientations

    // the daemon waits for sign-in before its first start and when asked to again
    Timer {
        id: signInFollower
        interval: 0
        running: true
        onTriggered: {
            if (pageStack.busy) {
                restart()
                return
            }
            var shown = pageStack.currentPage && pageStack.currentPage.objectName === "signIn"
            if (daemon.signInStep !== "" && !shown)
                pageStack.push(Qt.resolvedUrl("pages/SignInPage.qml"))
            else if (daemon.signInStep === "" && shown)
                pageStack.pop()
        }
    }

    Connections {
        target: daemon
        onSignInChanged: signInFollower.restart()
        onActionFailed: {
            var texts = {
                rename: qsTr("Could not rename the group"),
                add: qsTr("Could not add the person"),
                addUnreachable: qsTr("This number or email cannot get iMessages"),
                remove: qsTr("Could not remove the person"),
                leave: qsTr("Could not leave the group"),
                photo: qsTr("Could not change the group photo")
            }
            Notices.show(texts[action] || qsTr("Could not change the group"), Notice.Long)
        }
    }

    Connections {
        target: appService
        onOpenChatRequested: {
            pageStack.clear()
            pageStack.push(Qt.resolvedUrl("pages/ChatsPage.qml"), {}, PageStackAction.Immediate)
            pageStack.push(Qt.resolvedUrl("pages/ConversationPage.qml"),
                           { chatId: chatId, title: chatModel.titleOf(chatId) }, PageStackAction.Immediate)
            appWindow.activate()
        }
    }
}
