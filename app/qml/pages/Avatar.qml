import QtQuick 2.0
import QtGraphicalEffects 1.0
import Sailfish.Silica 1.0

// A round picture of a person or group, or their initial on a colour of their own.
Item {
    id: avatar

    property string source
    property string name
    property bool group

    readonly property string initial: {
        var first = name.trim().charAt(0)
        return first.toUpperCase() !== first.toLowerCase() ? first.toUpperCase() : ""
    }

    function hue(text) {
        var sum = 0
        for (var i = 0; i < text.length; i++)
            sum = (sum * 31 + text.charCodeAt(i)) % 360
        return sum / 360
    }

    Rectangle {
        anchors.fill: parent
        visible: picture.status !== Image.Ready
        radius: width / 2
        color: Qt.hsla(avatar.hue(avatar.name), 0.5, 0.45, 1)

        Label {
            anchors.centerIn: parent
            visible: avatar.initial !== "" && !avatar.group
            text: avatar.initial
            font.pixelSize: parent.height * 0.45
            color: "white"
        }

        Icon {
            anchors.centerIn: parent
            visible: avatar.initial === "" || avatar.group
            width: Math.min(implicitWidth, parent.width * 0.6)
            height: width
            source: avatar.group ? "image://theme/icon-m-users" : "image://theme/icon-m-person"
            color: "white"
        }
    }

    Image {
        id: picture
        anchors.fill: parent
        visible: false
        source: avatar.source
        sourceSize.width: width
        sourceSize.height: height
        fillMode: Image.PreserveAspectCrop
        asynchronous: true
    }

    Rectangle {
        id: mask
        anchors.fill: parent
        visible: false
        radius: width / 2
    }

    OpacityMask {
        anchors.fill: parent
        visible: picture.status === Image.Ready
        source: picture
        maskSource: mask
    }
}
