#ifndef MESSAGEMODEL_H
#define MESSAGEMODEL_H

#include <QAbstractListModel>

#include "daemon.h"

// Messages of one chat, newest first (row 0), for a bottom-to-top ListView.
class MessageModel : public QAbstractListModel
{
    Q_OBJECT
    Q_PROPERTY(Daemon *service READ service WRITE setService NOTIFY serviceChanged)
    Q_PROPERTY(QString chatId READ chatId WRITE setChatId NOTIFY chatIdChanged)

public:
    enum Roles {
        MessageIdRole = Qt::UserRole + 1, TextRole, TimeRole, StatusRole, MineRole,
        SenderNameRole, FirstOfRunRole, ReactionsRole, ReplyToRole, ReplyTextRole, EditedRole, UnsentRole,
        AttachmentsRole, KindRole
    };

    explicit MessageModel(QObject *parent = 0);

    int rowCount(const QModelIndex &parent = QModelIndex()) const;
    QVariant data(const QModelIndex &index, int role) const;
    QHash<int, QByteArray> roleNames() const;

    Daemon *service() const { return m_daemon; }
    void setService(Daemon *daemon);
    QString chatId() const { return m_chatId; }
    void setChatId(const QString &chatId);

    Q_INVOKABLE void loadMore();
    // Loads older pages until the message is in; returns its row, or -1.
    Q_INVOKABLE int loadUntil(const QString &messageId);

signals:
    void serviceChanged();
    void chatIdChanged();

private slots:
    void reload();
    void onReceived(const QString &chatId, const QString &messageId);
    void onStatus(const QString &chatId, const QString &messageId, uint status);

private:
    Daemon *m_daemon;
    QString m_chatId;
    QList<MessageEntry> m_messages;
    bool m_atStart;
};

#endif
