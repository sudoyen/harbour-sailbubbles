#include "messagemodel.h"

#include <QDateTime>

QString displayText(QString text);

static const uint PageSize = 50;

MessageModel::MessageModel(QObject *parent)
    : QAbstractListModel(parent)
    , m_daemon(0)
    , m_atStart(false)
{
}

int MessageModel::rowCount(const QModelIndex &parent) const
{
    return parent.isValid() ? 0 : m_messages.size();
}

QVariant MessageModel::data(const QModelIndex &index, int role) const
{
    if (!index.isValid() || index.row() >= m_messages.size())
        return QVariant();
    const MessageEntry &m = m_messages.at(index.row());
    switch (role) {
    case MessageIdRole: return m.id;
    // attachments show as themselves, not as their placeholder in the text
    case TextRole: return m.attachments.isEmpty() ? displayText(m.text)
                                                  : QString(m.text).remove(QChar(0xfffc)).simplified();
    case TimeRole: return QDateTime::fromMSecsSinceEpoch(m.ts);
    case StatusRole: return m.status;
    case MineRole: return m.sender.isEmpty();
    case SenderNameRole: return m.senderName;
    // rows run newest first, so the earlier message is the next row
    case FirstOfRunRole:
        return index.row() + 1 >= m_messages.size() || m_messages.at(index.row() + 1).sender != m.sender
            || !m_messages.at(index.row() + 1).kind.isEmpty();
    case ReactionsRole: {
        QVariantList reactions;
        foreach (const ReactionEntry &r, m.reactions) {
            QVariantMap item;
            item.insert("who", r.who);
            item.insert("kind", r.kind);
            reactions.append(item);
        }
        return reactions;
    }
    case ReplyToRole: return m.replyTo;
    case ReplyTextRole: return displayText(m.replyText);
    case EditedRole: return m.edited;
    case UnsentRole: return m.unsent;
    case KindRole: return m.kind;
    case AttachmentsRole: {
        QVariantList attachments;
        foreach (const AttachmentEntry &a, m.attachments) {
            QVariantMap item;
            item.insert("id", a.id);
            item.insert("name", a.name);
            item.insert("mime", a.mime);
            item.insert("size", a.size);
            item.insert("path", a.path);
            attachments.append(item);
        }
        return attachments;
    }
    }
    return QVariant();
}

QHash<int, QByteArray> MessageModel::roleNames() const
{
    QHash<int, QByteArray> roles;
    roles[MessageIdRole] = "messageId";
    roles[TextRole] = "text";
    roles[TimeRole] = "time";
    roles[StatusRole] = "status";
    roles[MineRole] = "mine";
    roles[SenderNameRole] = "senderName";
    roles[FirstOfRunRole] = "firstOfRun";
    roles[ReactionsRole] = "reactions";
    roles[ReplyToRole] = "replyTo";
    roles[ReplyTextRole] = "replyText";
    roles[EditedRole] = "edited";
    roles[UnsentRole] = "unsent";
    roles[AttachmentsRole] = "attachments";
    roles[KindRole] = "kind";
    return roles;
}

void MessageModel::setService(Daemon *daemon)
{
    if (daemon == m_daemon)
        return;
    if (m_daemon)
        disconnect(m_daemon, 0, this, 0);
    m_daemon = daemon;
    if (m_daemon) {
        connect(m_daemon, SIGNAL(messageReceived(QString,QString)), this, SLOT(onReceived(QString,QString)));
        connect(m_daemon, SIGNAL(messageStatus(QString,QString,uint)), this, SLOT(onStatus(QString,QString,uint)));
        connect(m_daemon, SIGNAL(availableChanged()), this, SLOT(reload()));
    }
    emit serviceChanged();
    reload();
}

void MessageModel::setChatId(const QString &chatId)
{
    if (chatId == m_chatId)
        return;
    m_chatId = chatId;
    emit chatIdChanged();
    reload();
}

void MessageModel::reload()
{
    // keep as many rows as were loaded, so older pages stay put
    const uint wanted = qMax<uint>(PageSize, m_messages.size());
    beginResetModel();
    m_messages.clear();
    if (m_daemon && !m_chatId.isEmpty())
        m_messages = m_daemon->messages(m_chatId, 0, wanted);
    m_atStart = (uint)m_messages.size() < wanted;
    endResetModel();
}

void MessageModel::loadMore()
{
    if (m_atStart || !m_daemon || m_messages.isEmpty())
        return;
    QList<MessageEntry> older = m_daemon->messages(m_chatId, m_messages.last().ts, PageSize);
    m_atStart = (uint)older.size() < PageSize;
    if (older.isEmpty())
        return;
    beginInsertRows(QModelIndex(), m_messages.size(), m_messages.size() + older.size() - 1);
    m_messages.append(older);
    endInsertRows();
}

int MessageModel::loadUntil(const QString &messageId)
{
    for (;;) {
        for (int i = 0; i < m_messages.size(); i++)
            if (m_messages.at(i).id == messageId)
                return i;
        if (m_atStart)
            return -1;
        const int loaded = m_messages.size();
        loadMore();
        if (m_messages.size() == loaded)
            return -1;
    }
}

void MessageModel::onReceived(const QString &chatId, const QString &)
{
    if (chatId == m_chatId)
        reload();
}

void MessageModel::onStatus(const QString &chatId, const QString &messageId, uint status)
{
    if (chatId != m_chatId)
        return;
    for (int i = 0; i < m_messages.size(); ++i) {
        if (m_messages.at(i).id == messageId) {
            m_messages[i].status = status;
            emit dataChanged(index(i), index(i), QVector<int>() << StatusRole);
            return;
        }
    }
    // a message sent from this device but not loaded yet
    reload();
}
