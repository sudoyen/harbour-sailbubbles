#ifndef CHATMODEL_H
#define CHATMODEL_H

#include <QAbstractListModel>

#include "daemon.h"

class ChatModel : public QAbstractListModel
{
    Q_OBJECT
    Q_PROPERTY(int totalUnread READ totalUnread NOTIFY totalUnreadChanged)
    Q_PROPERTY(int count READ count NOTIFY countChanged)

public:
    enum Roles { ChatIdRole = Qt::UserRole + 1, TitleRole, LastTextRole, LastTimeRole, UnreadRole, PinnedRole,
                 ParticipantsRole, MutedRole, IconRole, LeftRole, NameRole, ArchivedRole, KnownRole,
                 GroupRole };

    // Which chats the model holds: the main list, archived ones, or chats with
    // unknown senders when those are filtered out of the main list.
    enum View { Main, Archived, Unknown };

    explicit ChatModel(Daemon *daemon, View view = Main, QObject *parent = 0);

    int rowCount(const QModelIndex &parent = QModelIndex()) const;
    QVariant data(const QModelIndex &index, int role) const;
    QHash<int, QByteArray> roleNames() const;
    int totalUnread() const { return m_totalUnread; }
    int count() const { return m_chats.size(); }
    Q_INVOKABLE QString titleOf(const QString &chatId) const;
    // the chat's roles by name, from any view, or an empty map
    Q_INVOKABLE QVariantMap chat(const QString &chatId) const;

public slots:
    void reload();

signals:
    void totalUnreadChanged();
    void countChanged();

private:
    bool shown(const ChatEntry &c, bool filterUnknown) const;
    QVariant value(const ChatEntry &c, int role) const;

    Daemon *m_daemon;
    View m_view;
    QList<ChatEntry> m_all;
    QList<ChatEntry> m_chats;
    int m_totalUnread;
};

#endif
