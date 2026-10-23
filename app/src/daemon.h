#ifndef DAEMON_H
#define DAEMON_H

#include <QDBusArgument>
#include <QDBusMessage>
#include <QList>
#include <QMap>
#include <QObject>
#include <QStringList>
#include <QVariantMap>

class QDBusServiceWatcher;

struct ChatEntry {
    QString id;
    QStringList participants;  // other than us
    QString title;
    QString lastText;
    qint64 lastTs;
    uint unread;
    bool pinned;
    bool muted;
    QString icon;  // group photo path, or empty
    bool left;
    QString name;  // a group's own name, or empty
    bool archived;
    bool known;  // a group, or someone in the contacts
    bool group;  // stays one when down to two people
};

struct ReactionEntry {
    QString who;  // empty for our own reaction
    QString kind;
};

struct AttachmentEntry {
    QString id;
    QString name;
    QString mime;
    qint64 size;
    QString path;  // empty until downloaded
};

struct MessageEntry {
    QString id;
    QString sender;
    QString senderName;
    QString text;
    qint64 ts;
    uint status;
    QList<ReactionEntry> reactions;
    QString replyTo;
    QString replyText;
    bool edited;
    bool unsent;
    QList<AttachmentEntry> attachments;
    QString kind;  // empty for a message, else a group event
};

Q_DECLARE_METATYPE(ChatEntry)
Q_DECLARE_METATYPE(QList<ChatEntry>)
Q_DECLARE_METATYPE(ReactionEntry)
Q_DECLARE_METATYPE(QList<ReactionEntry>)
Q_DECLARE_METATYPE(AttachmentEntry)
Q_DECLARE_METATYPE(QList<AttachmentEntry>)
Q_DECLARE_METATYPE(MessageEntry)
Q_DECLARE_METATYPE(QList<MessageEntry>)
typedef QMap<QString, QString> NameMap;
Q_DECLARE_METATYPE(NameMap)
typedef QMap<QString, bool> SettingMap;
Q_DECLARE_METATYPE(SettingMap)

QDBusArgument &operator<<(QDBusArgument &arg, const ChatEntry &c);
const QDBusArgument &operator>>(const QDBusArgument &arg, ChatEntry &c);
QDBusArgument &operator<<(QDBusArgument &arg, const ReactionEntry &r);
const QDBusArgument &operator>>(const QDBusArgument &arg, ReactionEntry &r);
QDBusArgument &operator<<(QDBusArgument &arg, const AttachmentEntry &a);
const QDBusArgument &operator>>(const QDBusArgument &arg, AttachmentEntry &a);
QDBusArgument &operator<<(QDBusArgument &arg, const MessageEntry &m);
const QDBusArgument &operator>>(const QDBusArgument &arg, MessageEntry &m);

// Client for org.sailbubbles.Daemon on the session bus.
class Daemon : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool available READ available NOTIFY availableChanged)
    // set while the daemon waits for sign-in instead of running: relay,
    // account, deviceCode, phone, smsCode, working or failed
    Q_PROPERTY(QString signInStep READ signInStep NOTIFY signInChanged)
    Q_PROPERTY(QString signInDetail READ signInDetail NOTIFY signInChanged)

public:
    enum Status { Received, Sending, Sent, Delivered, Read, Failed };
    Q_ENUM(Status)

    explicit Daemon(QObject *parent = 0);

    bool available() const { return m_available; }
    QString signInStep() const { return m_signInStep; }
    QString signInDetail() const { return m_signInDetail; }
    // Answers the current sign-in step; returns the daemon's objection, or an empty string.
    Q_INVOKABLE QString answerSignIn(const QStringList &values);
    // The daemon starts over waiting for sign-in.
    Q_INVOKABLE bool signInAgain();
    QList<ChatEntry> chats() const;
    QList<MessageEntry> messages(const QString &chatId, qint64 before, uint limit) const;

    // Returns the chat id, or an empty string (and emits sendFailed).
    Q_INVOKABLE QString send(const QStringList &to, const QString &text, const QString &replyTo = QString());
    // Sends a file from the user's folders as a message of its own; as send().
    Q_INVOKABLE QString sendFile(const QStringList &to, const QString &path, const QString &mime,
                                 const QString &replyTo = QString());
    Q_INVOKABLE void markRead(const QString &chatId);
    Q_INVOKABLE bool markUnread(const QString &chatId);
    Q_INVOKABLE bool setPinned(const QString &chatId, bool pinned);
    Q_INVOKABLE bool setMuted(const QString &chatId, bool muted);
    Q_INVOKABLE bool setArchived(const QString &chatId, bool archived);
    // earlier texts of an edited message, as maps of text and time, oldest first
    Q_INVOKABLE QVariantList editHistory(const QString &messageId) const;
    // maps of handle and name, us left out
    Q_INVOKABLE QVariantList participants(const QString &chatId) const;
    // maps of id, name, mime and path of the chat's photos and videos
    Q_INVOKABLE QVariantList chatMedia(const QString &chatId) const;
    // Group changes go out in the background; actionFailed tells when one could not.
    Q_INVOKABLE bool renameGroup(const QString &chatId, const QString &name);
    Q_INVOKABLE bool addParticipant(const QString &chatId, const QString &handle);
    Q_INVOKABLE bool removeParticipant(const QString &chatId, const QString &handle);
    Q_INVOKABLE bool leaveGroup(const QString &chatId);
    // Crops and scales the picture to Apple's 570x570 PNG; an empty path removes the photo.
    Q_INVOKABLE bool setGroupPhoto(const QString &chatId, const QString &picture);
    // maps of chatId, title, messageId ("" for a chat found by its title), text, time
    Q_INVOKABLE QVariantList search(const QString &term) const;
    // text, and files as maps of path, mime, name; files gone since are left out
    Q_INVOKABLE QVariantMap draft(const QString &chatId) const;
    Q_INVOKABLE bool setDraft(const QString &chatId, const QString &text, const QStringList &paths);
    Q_INVOKABLE bool deleteChat(const QString &chatId);
    Q_INVOKABLE bool react(const QString &messageId, const QString &kind);
    Q_INVOKABLE bool edit(const QString &messageId, const QString &text);
    Q_INVOKABLE bool unsend(const QString &messageId);
    Q_INVOKABLE bool deleteMessage(const QString &messageId);
    Q_INVOKABLE void typing(const QString &chatId, bool typing);
    Q_INVOKABLE bool download(const QString &attachmentId);
    bool setNames(const QMap<QString, QString> &names);
    Q_INVOKABLE QVariantMap settings() const;
    // push, registration (state words from the daemon) and renewAt (QDateTime, invalid if unknown)
    Q_INVOKABLE QVariantMap status() const;
    Q_INVOKABLE bool setSetting(const QString &key, bool value);

signals:
    void availableChanged();
    void signInChanged();
    void chatsChanged();
    void messageReceived(const QString &chatId, const QString &messageId);
    void messageStatus(const QString &chatId, const QString &messageId, uint status);
    void chatRead(const QString &chatId);
    void typingChanged(const QString &chatId, bool typing);
    void statusChanged();
    void sendFailed(const QString &error);
    // action: rename, add, addUnreachable, remove, leave or photo
    void actionFailed(const QString &chatId, const QString &action);

private slots:
    void onReceived(const QString &chatId, const QString &messageId);
    void onStatus(const QString &chatId, const QString &messageId, uint status);
    void onChatRead(const QString &chatId);
    void onTypingChanged(const QString &chatId, bool typing);
    void setAvailable(bool available);
    void onSignInStep(const QString &step, const QString &detail);

private:
    QString sent(const QDBusMessage &reply);

    QDBusServiceWatcher *m_watcher;
    bool m_available;
    QString m_signInStep;
    QString m_signInDetail;
};

#endif
