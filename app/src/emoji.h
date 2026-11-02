#ifndef EMOJI_H
#define EMOJI_H

#include <QFile>
#include <QHash>
#include <QObject>
#include <QQuickImageProvider>

// Qt 5.6 cannot draw the colour glyphs of the system emoji font (SVG in
// OpenType), only its black outlines. This draws the SVG glyphs itself: text
// goes through styled(), which turns emoji into images from the "emoji"
// image provider.
class Emoji : public QObject, public QQuickImageProvider
{
    Q_OBJECT

public:
    explicit Emoji(const QString &fontPath = "/usr/share/fonts/twemoji/TwitterColorEmoji-SVGinOT.ttf",
                   QObject *parent = 0);

    // Escaped text for Text.StyledText, emoji as images of the given size.
    Q_INVOKABLE QString styled(const QString &text, int size) const;

    QImage requestImage(const QString &id, QSize *size, const QSize &requestedSize);

private:
    struct Doc { quint32 offset, length; };

    QString emojiRun(const QString &run, int size) const;
    QByteArray svgFor(quint32 glyph) const;

    QFile m_file;
    const uchar *m_data;
    QHash<quint32, Doc> m_docs;
    QString m_family;
    int m_unitsPerEm;
};

#endif
