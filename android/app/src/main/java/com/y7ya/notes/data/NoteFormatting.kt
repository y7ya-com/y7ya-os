package com.y7ya.notes.data

import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.time.format.TextStyle
import java.time.temporal.ChronoUnit
import java.util.Locale
import uniffi.notes_ffi.NoteRecord

private fun NoteRecord.lines() = body.lineSequence().map { it.trim() }.filter { it.isNotEmpty() }

/** The first non-empty line. */
val NoteRecord.title: String get() = lines().firstOrNull() ?: "New Note"

/** The second non-empty line. */
val NoteRecord.preview: String get() = lines().drop(1).firstOrNull() ?: "No additional text"

private fun localDate(secs: Long): LocalDate = Instant.ofEpochSecond(secs).atZone(ZoneId.systemDefault()).toLocalDate()

private fun daysAgo(secs: Long, today: LocalDate): Long = ChronoUnit.DAYS.between(localDate(secs), today)

private val timeFormat get() = DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT).withLocale(Locale.getDefault())

/** List date: time if today, "Yesterday", weekday within a week, else M/d/yy. */
fun listDate(secs: Long, today: LocalDate = LocalDate.now()): String {
    val time = Instant.ofEpochSecond(secs).atZone(ZoneId.systemDefault())
    return when (val days = daysAgo(secs, today)) {
        0L -> time.format(timeFormat)
        1L -> "Yesterday"
        in 2L..6L -> time.dayOfWeek.getDisplayName(TextStyle.FULL, Locale.getDefault())
        else -> if (days < 0) time.format(timeFormat) else time.format(DateTimeFormatter.ofPattern("M/d/yy"))
    }
}

/** Editor header date, e.g. "November 14, 2026 at 5:20 PM". */
fun editorDate(secs: Long): String {
    val time = Instant.ofEpochSecond(secs).atZone(ZoneId.systemDefault())
    val date = time.format(DateTimeFormatter.ofPattern("MMMM d, yyyy", Locale.getDefault()))
    return "$date at ${time.format(timeFormat)}"
}

data class NoteSection(val title: String, val notes: List<NoteRecord>)

/** Groups notes the way Notes does: Pinned, Today, Yesterday, Previous 7/30 Days, months, years. */
fun sectionNotes(notes: List<NoteRecord>, pinnedSection: Boolean, today: LocalDate = LocalDate.now()): List<NoteSection> {
    val sorted = notes.sortedByDescending { it.modified }
    val sections = LinkedHashMap<String, MutableList<NoteRecord>>()
    for (note in sorted) {
        val key = when {
            pinnedSection && note.pinned -> "Pinned"
            else -> {
                val days = daysAgo(note.modified, today)
                val date = localDate(note.modified)
                when {
                    days <= 0 -> "Today"
                    days == 1L -> "Yesterday"
                    days <= 7 -> "Previous 7 Days"
                    days <= 30 -> "Previous 30 Days"
                    date.year == today.year -> date.month.getDisplayName(TextStyle.FULL_STANDALONE, Locale.getDefault())
                    else -> date.year.toString()
                }
            }
        }
        sections.getOrPut(key) { mutableListOf() }.add(note)
    }
    // "Pinned" is inserted where its first note appeared; move it to the top.
    return sections.entries
        .sortedBy { if (it.key == "Pinned") 0 else 1 }
        .map { NoteSection(it.key, it.value) }
}
