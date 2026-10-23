// Shared by the pages; not a .pragma library, so qsTr resolves.

var kinds = ["heart", "like", "dislike", "laugh", "emphasize", "question"]

function label(kind) {
    switch (kind) {
    case "heart": return qsTr("Loved")
    case "like": return qsTr("Liked")
    case "dislike": return qsTr("Disliked")
    case "laugh": return qsTr("Laughed")
    case "emphasize": return qsTr("Emphasized")
    case "question": return qsTr("Questioned")
    }
    return kind.indexOf("emoji:") === 0 ? kind.substring(6) : kind
}
