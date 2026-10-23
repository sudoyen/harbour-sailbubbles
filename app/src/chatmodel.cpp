#include "chatmodel.h"

#include <QDateTime>

// The daemon marks attachments in text with U+FFFC.
QString displayText(QString text)
{
    return text.replace(QChar(0xfffc), QObject::tr("[Attachment]")).simplified();
}

ChatModel::ChatModel(Daemon *daemon, View view, QObject *parent)
    : QAbstractListModel(parent)
    , m_daemon(daemon)
    , m_view(view)
    , m_totalUnread(0)
{
    connect(daemon, SIGNAL(chatsChanged()), this, SLOT(reload()));
    reload();
}

int ChatModel::rowCount(const QModelIndex &parent) const
{
    return parent.isValid() ? 0 : m_chats.size();
}

QVariant ChatModel::data(const QModelIndex &index, int role) const
{
    if (!index.isValid() || index.row() >= m_chats.size())
        return QVariant();
    return value(m_chats.at(index.row()), role);
}

QVariant ChatModel::value(const ChatEntry &c, int role) const
{
    switch (role) {
    case ChatIdRole: return c.id;
    case TitleRole: return c.title;
    case LastTextRole: return displayText(c.lastText);
    case LastTimeRole: return QDateTime::fromMSecsSinceEpoch(c.lastTs);
    case UnreadRole: return c.unread;
    case PinnedRole: return c.pinned;
    case ParticipantsRole: return c.participants;
    case MutedRole: return c.muted;
    case IconRole: return c.icon;
    case LeftRole: return c.left;
    case NameRole: return c.name;
    case ArchivedRole: return c.archived;
    case KnownRole: return c.known;
    case GroupRole: return c.group;
    }
    return QVariant();
}

QHash<int, QByteArray> ChatModel::roleNames() const
{
    QHash<int, QByteArray> roles;
    roles[ChatIdRole] = "chatId";
    roles[TitleRole] = "title";
    roles[LastTextRole] = "lastText";
    roles[LastTimeRole] = "lastTime";
    roles[UnreadRole] = "unread";
    roles[PinnedRole] = "pinned";
    roles[ParticipantsRole] = "participants";
    roles[MutedRole] = "muted";
    roles[IconRole] = "icon";
    roles[LeftRole] = "left";
    roles[NameRole] = "name";
    roles[ArchivedRole] = "archived";
    roles[KnownRole] = "known";
    roles[GroupRole] = "group";
    return roles;
}

QString ChatModel::titleOf(const QString &chatId) const
{
    foreach (const ChatEntry &c, m_chats)
        if (c.id == chatId)
            return c.title;
    return QString();
}

QVariantMap ChatModel::chat(const QString &chatId) const
{
    QVariantMap found;
    foreach (const ChatEntry &c, m_all) {
        if (c.id != chatId)
            continue;
        const QHash<int, QByteArray> roles = roleNames();
        for (QHash<int, QByteArray>::const_iterator r = roles.constBegin(); r != roles.constEnd(); ++r)
            found.insert(QString::fromUtf8(r.value()), value(c, r.key()));
    }
    return found;
}

bool ChatModel::shown(const ChatEntry &c, bool filterUnknown) const
{
    switch (m_view) {
    case Archived: return c.archived;
    case Unknown: return !c.archived && filterUnknown && !c.known;
    case Main: break;
    }
    return !c.archived && (!filterUnknown || c.known);
}

void ChatModel::reload()
{
    const int before = m_chats.size();
    beginResetModel();
    m_all = m_daemon->available() ? m_daemon->chats() : QList<ChatEntry>();
    const bool filterUnknown = m_daemon->available() && m_daemon->settings().value("filterUnknown").toBool();
    m_chats.clear();
    foreach (const ChatEntry &c, m_all)
        if (shown(c, filterUnknown))
            m_chats.append(c);
    endResetModel();
    if (m_chats.size() != before)
        emit countChanged();

    int total = 0;
    foreach (const ChatEntry &c, m_chats)
        total += c.unread;
    if (total != m_totalUnread) {
        m_totalUnread = total;
        emit totalUnreadChanged();
    }
}
