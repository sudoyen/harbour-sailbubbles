import QtQuick 2.0
import Sailfish.Silica 1.0
import harbour.sailbubbles 1.0

// How far a message of ours got: a clock, one tick when sent, two when
// delivered, two highlighted when read.
Row {
    id: ticks

    property int status
    property real size: Theme.iconSizeExtraSmall

    spacing: -size * 0.6

    Icon {
        width: ticks.size
        height: ticks.size
        sourceSize.width: ticks.size
        sourceSize.height: ticks.size
        source: ticks.status === Daemon.Sending ? "image://theme/icon-s-time"
              : ticks.status === Daemon.Failed ? "image://theme/icon-s-warning"
              : "image://theme/icon-m-accept"
        color: ticks.status === Daemon.Failed ? Theme.errorColor
             : ticks.status === Daemon.Read ? Theme.highlightColor : Theme.secondaryColor
    }

    Icon {
        visible: ticks.status === Daemon.Delivered || ticks.status === Daemon.Read
        width: ticks.size
        height: ticks.size
        sourceSize.width: ticks.size
        sourceSize.height: ticks.size
        source: "image://theme/icon-m-accept"
        color: ticks.status === Daemon.Read ? Theme.highlightColor : Theme.secondaryColor
    }
}
