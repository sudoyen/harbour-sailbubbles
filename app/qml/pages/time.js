// Shared by the pages; not a .pragma library, so Format and Formatter resolve.

function daysAgo(time) {
    var now = new Date()
    var today = new Date(now.getFullYear(), now.getMonth(), now.getDate())
    var day = new Date(time.getFullYear(), time.getMonth(), time.getDate())
    return Math.round((today - day) / 86400000)
}

function day(time) {
    var days = daysAgo(time)
    if (days > 0 && days < 7)
        return Format.formatDate(time, Formatter.WeekdayNameStandalone)
    if (time.getFullYear() === new Date().getFullYear())
        return Format.formatDate(time, Formatter.DateMediumWithoutYear)
    return Format.formatDate(time, Formatter.DateMedium)
}

// Clock time today, otherwise the day.
function short(time) {
    return daysAgo(time) === 0 ? Format.formatDate(time, Formatter.TimeValue) : day(time)
}

// Clock time today, otherwise day and clock time.
function full(time) {
    var clock = Format.formatDate(time, Formatter.TimeValue)
    return daysAgo(time) === 0 ? clock : day(time) + " " + clock
}
