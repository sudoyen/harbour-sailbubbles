#include "contactnames.h"

#include <QContact>
#include <QContactAvatar>
#include <QContactDisplayLabel>
#include <QContactEmailAddress>
#include <QContactFetchHint>
#include <QContactFilter>
#include <QContactSortOrder>
#include <QContactName>
#include <QContactPhoneNumber>
#include <QSet>

#include "daemon.h"

// iMessage handles are international ("tel:+4915112345678") while contacts are
// often saved in local form ("0151 12345678"), so long numbers match on their
// last digits.
static const int MatchDigits = 8;

static QString digitsOf(const QString &number)
{
    QString digits;
    foreach (const QChar &c, number)
        if (c.isDigit())
            digits.append(c);
    return digits;
}

bool samePhoneNumber(const QString &a, const QString &b)
{
    const QString x = digitsOf(a), y = digitsOf(b);
    if (x.isEmpty() || y.isEmpty())
        return false;
    if (x.size() < MatchDigits || y.size() < MatchDigits)
        return x == y;
    return x.right(MatchDigits) == y.right(MatchDigits);
}

static QString nameOf(const QContact &contact)
{
    QString label = contact.detail<QContactDisplayLabel>().label().trimmed();
    if (!label.isEmpty())
        return label;
    const QContactName name = contact.detail<QContactName>();
    return (name.firstName() + " " + name.lastName()).trimmed();
}

ContactNames::ContactNames(Daemon *daemon, const QString &managerUri, QObject *parent)
    : QObject(parent)
    , m_daemon(daemon)
    , m_manager(QContactManager::fromUri(managerUri, this))
{
    // chat and contact changes come in bursts
    m_timer.setSingleShot(true);
    m_timer.setInterval(1000);
    connect(&m_timer, SIGNAL(timeout()), this, SLOT(sync()));

    connect(m_manager, SIGNAL(dataChanged()), this, SLOT(scheduleSync()));
    connect(m_manager, SIGNAL(contactsAdded(QList<QContactId>)), this, SLOT(scheduleSync()));
    connect(m_manager, SIGNAL(contactsChanged(QList<QContactId>,QList<QContactDetail::DetailType>)),
            this, SLOT(scheduleSync()));
    connect(m_manager, SIGNAL(contactsRemoved(QList<QContactId>)), this, SLOT(scheduleSync()));
    if (m_daemon) {
        connect(m_daemon, SIGNAL(chatsChanged()), this, SLOT(scheduleSync()));
        connect(m_daemon, SIGNAL(availableChanged()), this, SLOT(daemonRestarted()));
        scheduleSync();
    }
}

QList<QContact> ContactNames::fetch() const
{
    QContactFetchHint hint;
    hint.setDetailTypesHint(QList<QContactDetail::DetailType>()
        << QContactDisplayLabel::Type << QContactName::Type << QContactAvatar::Type
        << QContactPhoneNumber::Type << QContactEmailAddress::Type);
    return m_manager->contacts(QContactFilter(), QList<QContactSortOrder>(), hint);
}

// Index of the next contact from `from` on with this handle, or -1. Our own
// card can carry our own handle, which is in every chat.
int ContactNames::find(const QList<QContact> &contacts, const QString &handle, int from) const
{
    const bool phone = handle.startsWith("tel:");
    const QString value = handle.mid(handle.indexOf(':') + 1);
    const QContactId self = m_manager->selfContactId();
    for (int i = from; i < contacts.size(); i++) {
        const QContact &contact = contacts.at(i);
        if (contact.id() == self)
            continue;
        if (phone) {
            foreach (const QContactPhoneNumber &n, contact.details<QContactPhoneNumber>())
                if (samePhoneNumber(n.number(), value))
                    return i;
        } else {
            foreach (const QContactEmailAddress &e, contact.details<QContactEmailAddress>())
                if (e.emailAddress().compare(value, Qt::CaseInsensitive) == 0)
                    return i;
        }
    }
    return -1;
}

QMap<QString, QString> ContactNames::namesFor(const QStringList &handles) const
{
    const QList<QContact> contacts = fetch();
    QMap<QString, QString> names;
    foreach (const QString &handle, handles) {
        for (int i = find(contacts, handle, 0); i >= 0; i = find(contacts, handle, i + 1)) {
            const QString name = nameOf(contacts.at(i));
            if (!name.isEmpty()) {
                names.insert(handle, name);
                break;
            }
        }
    }
    return names;
}

QVariantMap ContactNames::avatarsFor(const QStringList &handles) const
{
    const QList<QContact> contacts = fetch();
    QVariantMap avatars;
    foreach (const QString &handle, handles) {
        for (int i = find(contacts, handle, 0); i >= 0; i = find(contacts, handle, i + 1)) {
            const QUrl url = contacts.at(i).detail<QContactAvatar>().imageUrl();
            if (!url.isEmpty()) {
                avatars.insert(handle, url.toString());
                break;
            }
        }
    }
    return avatars;
}

void ContactNames::sync()
{
    if (!m_daemon || !m_daemon->available())
        return;
    QSet<QString> handles;
    foreach (const ChatEntry &chat, m_daemon->chats())
        foreach (const QString &handle, chat.participants)
            handles.insert(handle);
    const QMap<QString, QString> names = namesFor(handles.toList());
    if (names != m_sent && m_daemon->setNames(names))
        m_sent = names;
    const QVariantMap avatars = avatarsFor(handles.toList());
    if (avatars != m_avatars) {
        m_avatars = avatars;
        emit avatarsChanged();
    }
}

void ContactNames::scheduleSync()
{
    m_timer.start();
}

void ContactNames::daemonRestarted()
{
    m_sent.clear();
    scheduleSync();
}
