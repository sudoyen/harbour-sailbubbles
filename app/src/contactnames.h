#ifndef CONTACTNAMES_H
#define CONTACTNAMES_H

#include <QMap>
#include <QObject>
#include <QStringList>
#include <QTimer>
#include <QVariantMap>

#include <QContactManager>

QTCONTACTS_USE_NAMESPACE

class Daemon;

bool samePhoneNumber(const QString &a, const QString &b);

// Looks up chat partners in the phone's contacts and hands their names to the
// daemon, which cannot read the contacts database itself. Only handles that
// appear in chats are sent, never the whole address book. Their photos stay
// in the app, keyed by handle.
class ContactNames : public QObject
{
    Q_OBJECT
    Q_PROPERTY(QVariantMap avatars READ avatars NOTIFY avatarsChanged)

public:
    explicit ContactNames(Daemon *daemon,
                          const QString &managerUri = "qtcontacts:org.nemomobile.contacts.sqlite:",
                          QObject *parent = 0);

    QMap<QString, QString> namesFor(const QStringList &handles) const;
    QVariantMap avatarsFor(const QStringList &handles) const;
    QVariantMap avatars() const { return m_avatars; }

signals:
    void avatarsChanged();

public slots:
    void sync();

private slots:
    void scheduleSync();
    void daemonRestarted();

private:
    QList<QContact> fetch() const;
    int find(const QList<QContact> &contacts, const QString &handle, int from) const;

    Daemon *m_daemon;
    QContactManager *m_manager;
    QTimer m_timer;
    QMap<QString, QString> m_sent;
    QVariantMap m_avatars;
};

#endif
