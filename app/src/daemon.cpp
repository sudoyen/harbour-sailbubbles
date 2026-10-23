#include "daemon.h"

#include <QDBusConnection>
#include <QDBusConnectionInterface>
#include <QDBusMessage>
#include <QDBusMetaType>
#include <QDBusReply>
#include <QDBusServiceWatcher>
#include <QDateTime>
#include <QDebug>
#include <QDir>
#include <QFileInfo>
#include <QImage>
#include <QImageReader>
#include <QMimeDatabase>

static const char *Service = "org.sailbubbles.Daemon";
static const char *Path = "/org/sailbubbles/Daemon";
static const char *Interface = "org.sailbubbles.Daemon1";
static const char *SignInInterface = "org.sailbubbles.SignIn1";
// The daemon answers from its own database, so calls are quick.
static const int CallTimeoutMs = 5000;

QDBusArgument &operator<<(QDBusArgument &arg, const ChatEntry &c)
{
    arg.beginStructure();
    arg << c.id << c.participants << c.title << c.lastText << c.lastTs << c.unread << c.pinned << c.muted
        << c.icon << c.left << c.name << c.archived << c.known << c.group;
    arg.endStructure();
    return arg;
}

const QDBusArgument &operator>>(const QDBusArgument &arg, ChatEntry &c)
{
    arg.beginStructure();
    arg >> c.id >> c.participants >> c.title >> c.lastText >> c.lastTs >> c.unread >> c.pinned >> c.muted
        >> c.icon >> c.left >> c.name >> c.archived >> c.known >> c.group;
    arg.endStructure();
    return arg;
}

QDBusArgument &operator<<(QDBusArgument &arg, const ReactionEntry &r)
{
    arg.beginStructure();
    arg << r.who << r.kind;
    arg.endStructure();
    return arg;
}

const QDBusArgument &operator>>(const QDBusArgument &arg, ReactionEntry &r)
{
    arg.beginStructure();
    arg >> r.who >> r.kind;
    arg.endStructure();
    return arg;
}

QDBusArgument &operator<<(QDBusArgument &arg, const AttachmentEntry &a)
{
    arg.beginStructure();
    arg << a.id << a.name << a.mime << a.size << a.path;
    arg.endStructure();
    return arg;
}

const QDBusArgument &operator>>(const QDBusArgument &arg, AttachmentEntry &a)
{
    arg.beginStructure();
    arg >> a.id >> a.name >> a.mime >> a.size >> a.path;
    arg.endStructure();
    return arg;
}

QDBusArgument &operator<<(QDBusArgument &arg, const MessageEntry &m)
{
    arg.beginStructure();
    arg << m.id << m.sender << m.senderName << m.text << m.ts << m.status << m.reactions
        << m.replyTo << m.replyText << m.edited << m.unsent << m.attachments << m.kind;
    arg.endStructure();
    return arg;
}

const QDBusArgument &operator>>(const QDBusArgument &arg, MessageEntry &m)
{
    arg.beginStructure();
    arg >> m.id >> m.sender >> m.senderName >> m.text >> m.ts >> m.status >> m.reactions
        >> m.replyTo >> m.replyText >> m.edited >> m.unsent >> m.attachments >> m.kind;
    arg.endStructure();
    return arg;
}

static QDBusMessage call(const QString &method, const QList<QVariant> &args = QList<QVariant>(),
                         const char *interface = Interface)
{
    QDBusMessage msg = QDBusMessage::createMethodCall(Service, Path, interface, method);
    msg.setArguments(args);
    return QDBusConnection::sessionBus().call(msg, QDBus::Block, CallTimeoutMs);
}

Daemon::Daemon(QObject *parent)
    : QObject(parent)
    , m_watcher(new QDBusServiceWatcher(Service, QDBusConnection::sessionBus(),
                                        QDBusServiceWatcher::WatchForOwnerChange, this))
    , m_available(false)
{
    qDBusRegisterMetaType<ChatEntry>();
    qDBusRegisterMetaType<QList<ChatEntry> >();
    qDBusRegisterMetaType<ReactionEntry>();
    qDBusRegisterMetaType<QList<ReactionEntry> >();
    qDBusRegisterMetaType<AttachmentEntry>();
    qDBusRegisterMetaType<QList<AttachmentEntry> >();
    qDBusRegisterMetaType<MessageEntry>();
    qDBusRegisterMetaType<QList<MessageEntry> >();
    qDBusRegisterMetaType<NameMap>();
    qDBusRegisterMetaType<SettingMap>();

    QDBusConnection bus = QDBusConnection::sessionBus();
    bus.connect(Service, Path, Interface, "MessageReceived", this, SLOT(onReceived(QString,QString)));
    bus.connect(Service, Path, Interface, "MessageStatus", this, SLOT(onStatus(QString,QString,uint)));
    bus.connect(Service, Path, Interface, "ChatRead", this, SLOT(onChatRead(QString)));
    bus.connect(Service, Path, Interface, "ChatsChanged", this, SIGNAL(chatsChanged()));
    bus.connect(Service, Path, Interface, "TypingChanged", this, SLOT(onTypingChanged(QString,bool)));
    bus.connect(Service, Path, Interface, "StatusChanged", this, SIGNAL(statusChanged()));
    bus.connect(Service, Path, Interface, "ActionFailed", this, SIGNAL(actionFailed(QString,QString)));
    bus.connect(Service, Path, SignInInterface, "StepChanged", this, SLOT(onSignInStep(QString,QString)));
    connect(m_watcher, &QDBusServiceWatcher::serviceRegistered, [this]() { setAvailable(true); });
    connect(m_watcher, &QDBusServiceWatcher::serviceUnregistered, [this]() { setAvailable(false); });
    setAvailable(bus.interface() && bus.interface()->isServiceRegistered(Service));
}

QList<ChatEntry> Daemon::chats() const
{
    QDBusReply<QList<ChatEntry> > reply = call("Chats");
    if (!reply.isValid()) {
        qWarning() << "Chats failed:" << reply.error().name();
        return QList<ChatEntry>();
    }
    return reply.value();
}

QList<MessageEntry> Daemon::messages(const QString &chatId, qint64 before, uint limit) const
{
    QDBusReply<QList<MessageEntry> > reply = call("Messages", QList<QVariant>()
        << chatId << QVariant::fromValue(before) << QVariant::fromValue(limit));
    if (!reply.isValid()) {
        qWarning() << "Messages failed:" << reply.error().name();
        return QList<MessageEntry>();
    }
    return reply.value();
}

static bool ok(const QDBusMessage &reply, const char *method)
{
    if (reply.type() == QDBusMessage::ReplyMessage)
        return true;
    qWarning() << method << "failed:" << reply.errorName();
    return false;
}

QString Daemon::send(const QStringList &to, const QString &text, const QString &replyTo)
{
    return sent(call("Send", QList<QVariant>() << to << text << replyTo));
}

QString Daemon::sendFile(const QStringList &to, const QString &path, const QString &mime, const QString &replyTo)
{
    return sent(call("SendFile", QList<QVariant>() << to << path << mime << replyTo));
}

QString Daemon::sent(const QDBusMessage &reply)
{
    if (reply.type() != QDBusMessage::ReplyMessage || reply.arguments().size() != 2) {
        emit sendFailed(reply.errorMessage());
        return QString();
    }
    emit chatsChanged();
    return reply.arguments().at(0).toString();
}

void Daemon::markRead(const QString &chatId)
{
    QDBusMessage reply = call("MarkRead", QList<QVariant>() << chatId);
    if (reply.type() == QDBusMessage::ReplyMessage)
        emit chatsChanged();
}

bool Daemon::setNames(const QMap<QString, QString> &names)
{
    QDBusMessage reply = call("SetNames", QList<QVariant>() << QVariant::fromValue<NameMap>(names));
    if (reply.type() != QDBusMessage::ReplyMessage) {
        qWarning() << "SetNames failed:" << reply.errorName();
        return false;
    }
    emit chatsChanged();
    return true;
}

QVariantMap Daemon::settings() const
{
    QDBusReply<SettingMap> reply = call("Settings");
    QVariantMap settings;
    if (!reply.isValid()) {
        qWarning() << "Settings failed:" << reply.error().name();
        return settings;
    }
    const SettingMap values = reply.value();
    for (SettingMap::const_iterator it = values.constBegin(); it != values.constEnd(); ++it)
        settings.insert(it.key(), it.value());
    return settings;
}

QVariantMap Daemon::status() const
{
    QVariantMap status;
    QDBusMessage reply = call("Status");
    if (reply.type() != QDBusMessage::ReplyMessage || reply.arguments().size() != 3) {
        qWarning() << "Status failed:" << reply.errorName();
        return status;
    }
    status.insert("push", reply.arguments().at(0));
    status.insert("registration", reply.arguments().at(1));
    const qint64 renewAt = reply.arguments().at(2).toLongLong();
    status.insert("renewAt", renewAt > 0 ? QDateTime::fromMSecsSinceEpoch(renewAt) : QDateTime());
    return status;
}

QString displayText(QString text);

QVariantList Daemon::participants(const QString &chatId) const
{
    QVariantList people;
    QDBusMessage reply = call("Participants", QList<QVariant>() << chatId);
    if (reply.type() != QDBusMessage::ReplyMessage || reply.arguments().size() != 1) {
        qWarning() << "Participants failed:" << reply.errorName();
        return people;
    }
    const QDBusArgument arg = reply.arguments().at(0).value<QDBusArgument>();
    arg.beginArray();
    while (!arg.atEnd()) {
        QString handle, name;
        arg.beginStructure();
        arg >> handle >> name;
        arg.endStructure();
        QVariantMap person;
        person.insert("handle", handle);
        person.insert("name", name);
        people.append(person);
    }
    arg.endArray();
    return people;
}

QVariantList Daemon::chatMedia(const QString &chatId) const
{
    QVariantList media;
    QDBusMessage reply = call("ChatMedia", QList<QVariant>() << chatId);
    if (reply.type() != QDBusMessage::ReplyMessage || reply.arguments().size() != 1) {
        qWarning() << "ChatMedia failed:" << reply.errorName();
        return media;
    }
    const QDBusArgument arg = reply.arguments().at(0).value<QDBusArgument>();
    arg.beginArray();
    while (!arg.atEnd()) {
        QString id, name, mime, path;
        arg.beginStructure();
        arg >> id >> name >> mime >> path;
        arg.endStructure();
        QVariantMap item;
        item.insert("id", id);
        item.insert("name", name);
        item.insert("mime", mime);
        item.insert("path", path);
        media.append(item);
    }
    arg.endArray();
    return media;
}

bool Daemon::renameGroup(const QString &chatId, const QString &name)
{
    return ok(call("RenameGroup", QList<QVariant>() << chatId << name), "RenameGroup");
}

bool Daemon::addParticipant(const QString &chatId, const QString &handle)
{
    return ok(call("AddParticipant", QList<QVariant>() << chatId << handle), "AddParticipant");
}

bool Daemon::removeParticipant(const QString &chatId, const QString &handle)
{
    return ok(call("RemoveParticipant", QList<QVariant>() << chatId << handle), "RemoveParticipant");
}

bool Daemon::leaveGroup(const QString &chatId)
{
    return ok(call("LeaveGroup", QList<QVariant>() << chatId), "LeaveGroup");
}

// Apple's size for a group photo
static const int GroupPhotoSize = 570;

bool Daemon::setGroupPhoto(const QString &chatId, const QString &picture)
{
    if (picture.isEmpty())
        return ok(call("SetGroupPhoto", QList<QVariant>() << chatId << QString()), "SetGroupPhoto");
    QImageReader reader(picture);
    reader.setAutoTransform(true);
    const QImage image = reader.read();
    if (image.isNull())
        return false;
    const QImage scaled = image.scaled(GroupPhotoSize, GroupPhotoSize, Qt::KeepAspectRatioByExpanding,
                                       Qt::SmoothTransformation);
    const QImage square = scaled.copy((scaled.width() - GroupPhotoSize) / 2, (scaled.height() - GroupPhotoSize) / 2,
                                      GroupPhotoSize, GroupPhotoSize);
    // the one folder Sailjail lets us write that the daemon takes a group photo from
    const QDir cache(QDir::homePath() + "/.cache/org.sailbubbles/harbour-sailbubbles");
    if (!cache.mkpath(".") || !square.save(cache.filePath("group-photo.png"), "PNG"))
        return false;
    return ok(call("SetGroupPhoto", QList<QVariant>() << chatId << cache.filePath("group-photo.png")), "SetGroupPhoto");
}

QVariantList Daemon::search(const QString &term) const
{
    QVariantList found;
    QDBusMessage reply = call("Search", QList<QVariant>() << term);
    if (reply.type() != QDBusMessage::ReplyMessage || reply.arguments().size() != 1) {
        qWarning() << "Search failed:" << reply.errorName();
        return found;
    }
    const QDBusArgument arg = reply.arguments().at(0).value<QDBusArgument>();
    arg.beginArray();
    while (!arg.atEnd()) {
        QString chatId, title, messageId, text;
        qint64 ts;
        arg.beginStructure();
        arg >> chatId >> title >> messageId >> text >> ts;
        arg.endStructure();
        QVariantMap result;
        result.insert("chatId", chatId);
        result.insert("title", title);
        result.insert("messageId", messageId);
        result.insert("text", displayText(text));
        result.insert("time", QDateTime::fromMSecsSinceEpoch(ts));
        found.append(result);
    }
    arg.endArray();
    return found;
}

QVariantMap Daemon::draft(const QString &chatId) const
{
    QVariantMap draft;
    QDBusMessage reply = call("Draft", QList<QVariant>() << chatId);
    if (reply.type() != QDBusMessage::ReplyMessage || reply.arguments().size() != 2) {
        qWarning() << "Draft failed:" << reply.errorName();
        return draft;
    }
    QMimeDatabase mimes;
    QVariantList files;
    foreach (const QString &path, reply.arguments().at(1).toStringList()) {
        const QFileInfo info(path);
        if (!info.isFile())
            continue;
        QVariantMap file;
        file.insert("path", path);
        file.insert("mime", mimes.mimeTypeForFile(info).name());
        file.insert("name", info.fileName());
        files.append(file);
    }
    draft.insert("text", reply.arguments().at(0).toString());
    draft.insert("files", files);
    return draft;
}

bool Daemon::setDraft(const QString &chatId, const QString &text, const QStringList &paths)
{
    return ok(call("SetDraft", QList<QVariant>() << chatId << text << paths), "SetDraft");
}

bool Daemon::setSetting(const QString &key, bool value)
{
    QDBusMessage reply = call("SetSetting", QList<QVariant>() << key << value);
    if (reply.type() != QDBusMessage::ReplyMessage)
        return false;
    // the unknown-sender filter moves chats between lists
    emit chatsChanged();
    return true;
}

bool Daemon::markUnread(const QString &chatId)
{
    return ok(call("MarkUnread", QList<QVariant>() << chatId), "MarkUnread");
}

bool Daemon::setPinned(const QString &chatId, bool pinned)
{
    return ok(call("SetPinned", QList<QVariant>() << chatId << pinned), "SetPinned");
}

bool Daemon::setArchived(const QString &chatId, bool archived)
{
    return ok(call("SetArchived", QList<QVariant>() << chatId << archived), "SetArchived");
}

QVariantList Daemon::editHistory(const QString &messageId) const
{
    QVariantList versions;
    QDBusMessage reply = call("EditHistory", QList<QVariant>() << messageId);
    if (reply.type() != QDBusMessage::ReplyMessage || reply.arguments().size() != 1) {
        qWarning() << "EditHistory failed:" << reply.errorName();
        return versions;
    }
    const QDBusArgument arg = reply.arguments().at(0).value<QDBusArgument>();
    arg.beginArray();
    while (!arg.atEnd()) {
        QString text;
        qint64 ts;
        arg.beginStructure();
        arg >> text >> ts;
        arg.endStructure();
        QVariantMap version;
        version.insert("text", text);
        version.insert("time", QDateTime::fromMSecsSinceEpoch(ts));
        versions.append(version);
    }
    arg.endArray();
    return versions;
}

bool Daemon::setMuted(const QString &chatId, bool muted)
{
    return ok(call("SetMuted", QList<QVariant>() << chatId << muted), "SetMuted");
}

bool Daemon::deleteChat(const QString &chatId)
{
    return ok(call("DeleteChat", QList<QVariant>() << chatId), "DeleteChat");
}

bool Daemon::react(const QString &messageId, const QString &kind)
{
    return ok(call("React", QList<QVariant>() << messageId << kind), "React");
}

bool Daemon::edit(const QString &messageId, const QString &text)
{
    return ok(call("Edit", QList<QVariant>() << messageId << text), "Edit");
}

bool Daemon::unsend(const QString &messageId)
{
    return ok(call("Unsend", QList<QVariant>() << messageId), "Unsend");
}

bool Daemon::deleteMessage(const QString &messageId)
{
    return ok(call("DeleteMessage", QList<QVariant>() << messageId), "DeleteMessage");
}

void Daemon::typing(const QString &chatId, bool typing)
{
    ok(call("Typing", QList<QVariant>() << chatId << typing), "Typing");
}

bool Daemon::download(const QString &attachmentId)
{
    return ok(call("Download", QList<QVariant>() << attachmentId), "Download");
}

void Daemon::onTypingChanged(const QString &chatId, bool typing)
{
    emit typingChanged(chatId, typing);
}

void Daemon::onReceived(const QString &chatId, const QString &messageId)
{
    emit messageReceived(chatId, messageId);
    emit chatsChanged();
}

void Daemon::onStatus(const QString &chatId, const QString &messageId, uint status)
{
    emit messageStatus(chatId, messageId, status);
}

void Daemon::onChatRead(const QString &chatId)
{
    emit chatRead(chatId);
    emit chatsChanged();
}

void Daemon::setAvailable(bool available)
{
    // the daemon offers sign-in under the same name, so ask which it does now
    QString step, detail;
    if (available) {
        QDBusMessage reply = call("State", QList<QVariant>(), SignInInterface);
        if (reply.type() == QDBusMessage::ReplyMessage && reply.arguments().size() == 2) {
            step = reply.arguments().at(0).toString();
            detail = reply.arguments().at(1).toString();
        }
    }
    onSignInStep(step, detail);
    if (available == m_available)
        return;
    m_available = available;
    emit availableChanged();
    emit chatsChanged();
    emit statusChanged();
}

void Daemon::onSignInStep(const QString &step, const QString &detail)
{
    if (step == m_signInStep && detail == m_signInDetail)
        return;
    m_signInStep = step;
    m_signInDetail = detail;
    emit signInChanged();
}

QString Daemon::answerSignIn(const QStringList &values)
{
    QDBusMessage reply = call("Answer", QList<QVariant>() << values, SignInInterface);
    return reply.type() == QDBusMessage::ReplyMessage ? QString() : reply.errorMessage();
}

bool Daemon::signInAgain()
{
    return ok(call("SignInAgain"), "SignInAgain");
}
