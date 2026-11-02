#include "emoji.h"

#include <QFontDatabase>
#include <QGlyphRun>
#include <QPainter>
#include <QRawFont>
#include <QSvgRenderer>
#include <QTextLayout>
#include <QtEndian>

static quint16 u16(const uchar *p) { return qFromBigEndian<quint16>(p); }
static quint32 u32(const uchar *p) { return qFromBigEndian<quint32>(p); }

static bool joiner(uint c)
{
    return c == 0x200d || c == 0xfe0f || c == 0x20e3 || (c >= 0xe0020 && c <= 0xe007f);
}

// Emoji_Presentation=Yes outside the supplementary planes: drawn as emoji
// even without a U+FE0F after them.
static bool emojiInBmp(uint c)
{
    static const uint ranges[][2] = {
        {0x231a, 0x231b}, {0x23e9, 0x23ec}, {0x23f0, 0x23f0}, {0x23f3, 0x23f3},
        {0x25fd, 0x25fe}, {0x2614, 0x2615}, {0x2648, 0x2653}, {0x267f, 0x267f},
        {0x2693, 0x2693}, {0x26a1, 0x26a1}, {0x26aa, 0x26ab}, {0x26bd, 0x26be},
        {0x26c4, 0x26c5}, {0x26ce, 0x26ce}, {0x26d4, 0x26d4}, {0x26ea, 0x26ea},
        {0x26f2, 0x26f3}, {0x26f5, 0x26f5}, {0x26fa, 0x26fa}, {0x26fd, 0x26fd},
        {0x2705, 0x2705}, {0x270a, 0x270b}, {0x2728, 0x2728}, {0x274c, 0x274c},
        {0x274e, 0x274e}, {0x2753, 0x2755}, {0x2757, 0x2757}, {0x2795, 0x2797},
        {0x27b0, 0x27b0}, {0x27bf, 0x27bf}, {0x2b1b, 0x2b1c}, {0x2b50, 0x2b50},
        {0x2b55, 0x2b55},
    };
    for (const auto &r : ranges)
        if (c >= r[0] && c <= r[1])
            return true;
    return false;
}

// Characters that are text unless U+FE0F or a keycap follows.
static bool textByDefault(uint c)
{
    return (c >= '0' && c <= '9') || c == '#' || c == '*' || c == 0xa9 || c == 0xae
        || (c >= 0x2000 && c <= 0x2bff) || c == 0x3030 || c == 0x303d || c == 0x3297 || c == 0x3299;
}

Emoji::Emoji(const QString &fontPath, QObject *parent)
    : QObject(parent), QQuickImageProvider(QQuickImageProvider::Image),
      m_file(fontPath), m_data(0), m_unitsPerEm(0)
{
    if (!m_file.open(QIODevice::ReadOnly) || !(m_data = m_file.map(0, m_file.size())))
        return;
    const qint64 size = m_file.size();
    const uchar *svg = 0;
    for (int i = 0, n = u16(m_data + 4); i < n && 28 + 16 * i <= size; i++) {
        const uchar *entry = m_data + 12 + 16 * i;
        const quint32 offset = u32(entry + 8);
        if (offset >= size)
            continue;
        if (!memcmp(entry, "SVG ", 4))
            svg = m_data + offset;
        else if (!memcmp(entry, "head", 4))
            m_unitsPerEm = u16(m_data + offset + 18);
    }
    if (!svg || !m_unitsPerEm)
        return;
    const uchar *index = svg + u32(svg + 2);
    for (int i = 0, n = u16(index); i < n; i++) {
        const uchar *record = index + 2 + 12 * i;
        const Doc doc = { quint32(index - m_data) + u32(record + 4), u32(record + 8) };
        if (doc.offset + doc.length > size)
            continue;
        for (uint glyph = u16(record); glyph <= u16(record + 2); glyph++)
            m_docs.insert(glyph, doc);
    }
    const QStringList families = QFontDatabase::applicationFontFamilies(QFontDatabase::addApplicationFont(fontPath));
    if (!families.isEmpty())
        m_family = families.first();
}

QString Emoji::styled(const QString &text, int size) const
{
    QVector<uint> chars = text.toUcs4();
    QString out, run;
    for (int i = 0; i < chars.size(); i++) {
        const uint c = chars[i];
        const uint next = i + 1 < chars.size() ? chars[i + 1] : 0;
        const bool afterJoiner = !run.isEmpty() && run.endsWith(QChar(0x200d));
        if ((c >= 0x1f000 && c <= 0x1faff) || emojiInBmp(c) || (!run.isEmpty() && joiner(c))
                || (textByDefault(c) && (next == 0xfe0f || next == 0x20e3 || afterJoiner))) {
            run += QString::fromUcs4(&c, 1);
            continue;
        }
        if (!run.isEmpty()) {
            out += emojiRun(run, size);
            run.clear();
        }
        out += c == '\n' ? QString("<br>") : QString::fromUcs4(&c, 1).toHtmlEscaped();
    }
    if (!run.isEmpty())
        out += emojiRun(run, size);
    return out;
}

// One image per glyph the emoji font shapes the run into, so sequences with
// joiners and flags come out as the font's single picture for them.
QString Emoji::emojiRun(const QString &run, int size) const
{
    if (m_docs.isEmpty() || m_family.isEmpty())
        return run.toHtmlEscaped();
    QFont font(m_family);
    font.setStyleStrategy(QFont::NoFontMerging);
    // the font's ligatures are keyed on sequences without variation selectors
    QTextLayout layout(QString(run).remove(QChar(0xfe0f)), font);
    layout.beginLayout();
    layout.createLine();
    layout.endLayout();
    QString images;
    foreach (const QGlyphRun &glyphs, layout.glyphRuns()) {
        const QVector<quint32> indexes = glyphs.glyphIndexes();
        const QVector<QPointF> advances = glyphs.rawFont().advancesForGlyphIndexes(indexes);
        for (int i = 0; i < indexes.size(); i++) {
            if (m_docs.contains(indexes[i]))
                images += QString("<img src=\"image://emoji/%1\" width=\"%2\" height=\"%2\" align=\"middle\">")
                          .arg(indexes[i]).arg(size);
            else if (indexes[i] == 0 || advances[i].x() > 0)
                return run.toHtmlEscaped();
        }
    }
    return images.isEmpty() ? run.toHtmlEscaped() : images;
}

QByteArray Emoji::svgFor(quint32 glyph) const
{
    const Doc doc = m_docs.value(glyph);
    return QByteArray::fromRawData(reinterpret_cast<const char *>(m_data + doc.offset), doc.length);
}

QImage Emoji::requestImage(const QString &id, QSize *size, const QSize &requestedSize)
{
    const QSize out = requestedSize.isValid() ? requestedSize : QSize(64, 64);
    if (size)
        *size = out;
    QImage image(out, QImage::Format_ARGB32_Premultiplied);
    image.fill(Qt::transparent);
    const quint32 glyph = id.toUInt();
    if (!m_docs.contains(glyph))
        return image;
    QSvgRenderer renderer(svgFor(glyph));
    // Twemoji draws each glyph in a square from 0.8 em above the baseline
    // to 0.2 em below it; the documents carry no viewBox of their own.
    renderer.setViewBox(QRectF(0, -0.8 * m_unitsPerEm, m_unitsPerEm, m_unitsPerEm));
    QPainter painter(&image);
    renderer.render(&painter);
    return image;
}
