@file:OptIn(ExperimentalMaterial3Api::class)

package nl.merel.app

import android.content.Intent
import android.net.Uri
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.CalendarMonth
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.EditCalendar
import androidx.compose.material.icons.automirrored.outlined.HelpOutline
import androidx.compose.material.icons.outlined.HighlightOff
import androidx.compose.material.icons.outlined.Notifications
import androidx.compose.material.icons.outlined.Place
import androidx.compose.material.icons.outlined.Repeat
import androidx.compose.material.icons.outlined.Schedule
import androidx.compose.material.icons.outlined.Today
import androidx.compose.material.icons.outlined.Videocam
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.DatePicker
import androidx.compose.material3.DatePickerDialog
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExposedDropdownMenuBox
import androidx.compose.material3.ExposedDropdownMenuDefaults
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MenuAnchorType
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TimePicker
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.rememberDatePickerState
import androidx.compose.material3.rememberTimePickerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import java.time.DayOfWeek
import java.time.Instant
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.LocalTime
import java.time.ZoneId
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.time.format.TextStyle
import java.time.temporal.TemporalAdjusters
import java.util.Locale
import kotlinx.coroutines.delay
import uniffi.merel_mobile.AppointmentForm
import uniffi.merel_mobile.AppointmentRow
import uniffi.merel_mobile.AttendeeAnswer
import uniffi.merel_mobile.AttendeeRow
import uniffi.merel_mobile.CoreException

private val zone: ZoneId get() = ZoneId.systemDefault()
private fun Long.local(): LocalDateTime = LocalDateTime.ofInstant(Instant.ofEpochSecond(this), zone)

/// Where [epoch] falls on [day]'s clock face, in hours from 0 to 24. Uses the wall clock, so a
/// day with a daylight saving change (23 or 25 hours long) still lines up with its hour labels.
private fun clockHours(epoch: Long, day: LocalDate): Float {
    val t = epoch.local()
    return when {
        t.toLocalDate().isBefore(day) -> 0f
        t.toLocalDate().isAfter(day) -> 24f
        else -> t.hour + t.minute / 60f + t.second / 3600f
    }
}
private fun LocalDateTime.epoch(): Long = atZone(zone).toEpochSecond()
private fun calendarColor(rgb: UInt) = Color(0xFF000000L or rgb.toLong())
private fun String.capitalizedFirst() = replaceFirstChar { it.titlecase(Locale.getDefault()) }
private val timeFormat: DateTimeFormatter get() = DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT)
private fun dayTitle(d: LocalDate) = d.format(DateTimeFormatter.ofPattern("EEEE d MMMM", Locale.getDefault())).capitalizedFirst()

/** The calendar: a week strip and the chosen day on a time line, an appointment's details on top. */
@Composable
fun CalendarTab(store: Store) {
    val open = store.openAppointment
    val form = store.editing
    when {
        form != null -> AppointmentEditor(store, form)
        open != null -> AppointmentScreen(store, open.first, open.second)
        else -> DayScreen(store)
    }
}

/** A new appointment on [day] at the next whole hour (nine o'clock on another day), an hour long. */
fun newAppointment(day: LocalDate): AppointmentForm {
    val now = LocalDateTime.now()
    val start = if (day == now.toLocalDate()) now.withMinute(0).withSecond(0).withNano(0).plusHours(1) else day.atTime(9, 0)
    val t = start.epoch()
    return AppointmentForm(null, null, "", t, t + 3600, false, "", "", 15u)
}

// The day

@Composable
private fun DayScreen(store: Store) {
    val h = LocalHelder.current
    val day = store.day
    @Suppress("UNUSED_VARIABLE") val revision = store.revision
    val list = store.appointments(day, day.plusDays(1))
    val allDay = list.filter { it.allDay }
    Box(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            TopAppBar(
                title = { Text(day.month.getDisplayName(TextStyle.FULL_STANDALONE, Locale.getDefault()).capitalizedFirst()) },
                actions = {
                    IconButton(onClick = { store.day = LocalDate.now() }) {
                        Icon(Icons.Outlined.Today, stringResource(R.string.today))
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = h.window),
            )
            WeekStrip(store)
            Row(Modifier.fillMaxWidth().padding(16.dp, 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(
                    dayTitle(day), style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold,
                    modifier = Modifier.semantics { heading() },
                )
                Spacer(Modifier.width(8.dp).weight(1f))
                Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    allDay.forEach { a -> AllDayChip(a) { store.openAppointment = a.id to a.start } }
                }
            }
            TimeLine(
                day, list.filter { !it.allDay },
                onOpen = { store.openAppointment = it.id to it.start },
                onSwipe = { forward -> store.day = day.plusDays(if (forward) 1 else -1) },
                modifier = Modifier.weight(1f).padding(12.dp, 0.dp, 12.dp, 8.dp)
                    .clip(RoundedCornerShape(12.dp)).background(h.card).border(1.dp, h.border, RoundedCornerShape(12.dp)),
            )
        }
        ExtendedFloatingActionButton(
            onClick = { store.editing = newAppointment(day) },
            icon = { Icon(Icons.Outlined.EditCalendar, contentDescription = null) },
            text = { Text(stringResource(R.string.new_appointment)) },
            containerColor = h.accent, contentColor = h.onAccent,
            modifier = Modifier.align(Alignment.BottomEnd).padding(24.dp),
        )
    }
}

/** The seven days of the chosen day's week, a dot under a busy day; swipe for another week. */
@Composable
private fun WeekStrip(store: Store) {
    val h = LocalHelder.current
    val day = store.day
    val first = day.with(TemporalAdjusters.previousOrSame(DayOfWeek.MONDAY))
    val busy = store.appointments(first, first.plusDays(7)).map { it.start.local().toLocalDate() }.toSet()
    val today = LocalDate.now()
    var drag by remember { mutableStateOf(0f) }
    Row(
        Modifier.fillMaxWidth().padding(8.dp, 0.dp).pointerInput(day) {
            detectHorizontalDragGestures(
                onDragStart = { drag = 0f },
                onDragEnd = { if (kotlin.math.abs(drag) > 60) store.day = day.plusWeeks(if (drag < 0) 1 else -1) },
                onHorizontalDrag = { _, dx -> drag += dx },
            )
        },
    ) {
        (0L until 7L).map { first.plusDays(it) }.forEach { d ->
            val chosen = d == day
            Column(
                Modifier.weight(1f).clip(RoundedCornerShape(8.dp)).clickable { store.day = d }.padding(vertical = 4.dp)
                    .semantics(mergeDescendants = true) { selected = chosen; contentDescription = dayTitle(d) },
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Text(d.dayOfWeek.getDisplayName(TextStyle.SHORT, Locale.getDefault()), color = h.dim, fontSize = 13.sp)
                Box(
                    Modifier.size(40.dp).clip(CircleShape).background(if (chosen) h.accent else Color.Transparent),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        d.dayOfMonth.toString(), fontSize = 20.sp,
                        color = if (chosen) h.onAccent else if (d == today) h.ink else h.text,
                    )
                }
                Box(Modifier.size(5.dp).clip(CircleShape).background(if (d in busy && !chosen) h.dim else Color.Transparent))
            }
        }
    }
}

@Composable
private fun AllDayChip(a: AppointmentRow, onOpen: () -> Unit) {
    val h = LocalHelder.current
    val c = calendarColor(a.color)
    val label = stringResource(R.string.all_day_label, a.title)
    Row(
        Modifier.height(IntrinsicSize.Min).clip(RoundedCornerShape(6.dp)).background(h.card).background(c.copy(alpha = 0.16f))
            .clickable(onClick = onOpen).semantics(mergeDescendants = true) { contentDescription = label },
    ) {
        Box(Modifier.width(3.dp).fillMaxHeight().background(c))
        Text(a.title, color = h.text, fontSize = 14.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(8.dp, 4.dp))
    }
}

/** An appointment with its place among those it overlaps: lane [lane] of [of]. */
data class Placed(val row: AppointmentRow, val lane: Int, val of: Int)

/** Puts overlapping appointments side by side, as the desktop's day view does. */
fun lanes(list: List<AppointmentRow>): List<Placed> {
    val out = mutableListOf<Placed>()
    val group = mutableListOf<Pair<AppointmentRow, Int>>()
    var groupEnd = Long.MIN_VALUE
    fun close() {
        val count = (group.maxOfOrNull { it.second } ?: 0) + 1
        group.forEach { out += Placed(it.first, it.second, count) }
        group.clear()
    }
    for (a in list.sortedWith(compareBy({ it.start }, { it.end }))) {
        if (a.start >= groupEnd) close()
        var lane = 0
        while (group.any { it.second == lane && it.first.end > a.start }) lane++
        group += a to lane
        groupEnd = maxOf(groupEnd, a.end)
    }
    close()
    return out
}

/** The day's hours with its appointments, and a red line at the current time today. */
@Composable
private fun TimeLine(
    day: LocalDate,
    list: List<AppointmentRow>,
    onOpen: (AppointmentRow) -> Unit,
    onSwipe: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    val h = LocalHelder.current
    val hour: Dp = 58.dp
    val gutter: Dp = 56.dp
    val top: Dp = 10.dp
    val scroll = rememberScrollState()
    val density = LocalDensity.current
    var now by remember { mutableStateOf(LocalDateTime.now()) }
    LaunchedEffect(Unit) {
        while (true) {
            delay(60_000)
            now = LocalDateTime.now()
        }
    }
    LaunchedEffect(day) {
        val first = list.firstOrNull()?.start?.local()?.hour ?: if (day == LocalDate.now()) LocalTime.now().hour else 8
        scroll.scrollTo(with(density) { (hour * (first - 1).coerceIn(0, 23)).roundToPx() })
    }
    var drag by remember { mutableStateOf(0f) }
    BoxWithConstraints(
        modifier.pointerInput(day) {
            detectHorizontalDragGestures(
                onDragStart = { drag = 0f },
                onDragEnd = { if (kotlin.math.abs(drag) > 80) onSwipe(drag < 0) },
                onHorizontalDrag = { _, dx -> drag += dx },
            )
        },
    ) {
        val width = maxWidth - gutter - 8.dp
        Box(Modifier.fillMaxWidth().verticalScroll(scroll).height(hour * 24 + top + 12.dp)) {
            for (hh in 0 until 24) {
                Row(Modifier.offset(y = top + hour * hh - 7.dp).fillMaxWidth().clearAndSetSemantics {}) {
                    Text(
                        "%02d:00".format(hh), color = h.dim, fontSize = 12.sp,
                        modifier = Modifier.width(gutter - 12.dp).padding(end = 4.dp), textAlign = androidx.compose.ui.text.style.TextAlign.End,
                    )
                    Box(Modifier.padding(start = 8.dp, top = 7.dp).fillMaxWidth().height(1.dp).background(h.border))
                }
            }
            for (p in lanes(list)) {
                val a = p.row
                val from = clockHours(a.start, day)
                val to = clockHours(a.end, day)
                val laneWidth = width / p.of
                val height = (hour * (to - from) - 2.dp).coerceAtLeast(26.dp)
                AppointmentBlock(
                    a, height,
                    Modifier.offset(x = gutter + laneWidth * p.lane, y = top + hour * from + 1.dp).width(laneWidth - 3.dp).height(height),
                    onOpen = { onOpen(a) },
                )
            }
            if (day == now.toLocalDate()) {
                val y = top + hour * clockHours(now.atZone(zone).toEpochSecond(), day)
                Row(Modifier.offset(x = gutter - 6.dp, y = y - 4.5.dp).fillMaxWidth().clearAndSetSemantics {}, verticalAlignment = Alignment.CenterVertically) {
                    Box(Modifier.size(9.dp).clip(CircleShape).background(h.flag))
                    Box(Modifier.fillMaxWidth().height(2.dp).background(h.flag))
                }
            }
        }
    }
}

@Composable
private fun AppointmentBlock(a: AppointmentRow, height: Dp, modifier: Modifier, onOpen: () -> Unit) {
    val h = LocalHelder.current
    val c = calendarColor(a.color)
    val context = LocalContext.current
    val online = stringResource(R.string.online_meeting)
    val place = a.location.ifEmpty { if (a.onlineUrl != null) online else "" }
    Row(
        modifier.clip(RoundedCornerShape(6.dp)).background(h.card).background(c.copy(alpha = 0.16f)).clickable(onClick = onOpen)
            .semantics(mergeDescendants = true) {},
    ) {
        Box(Modifier.width(4.dp).fillMaxHeight().background(c))
        Row(Modifier.padding(8.dp, 4.dp).fillMaxSize()) {
            Column(Modifier.weight(1f)) {
                if (height < 44.dp) {
                    Text(
                        "${a.title} · ${timeRange(a)}", color = h.text, fontSize = 14.sp, fontWeight = FontWeight.SemiBold,
                        maxLines = 1, overflow = TextOverflow.Ellipsis,
                    )
                } else {
                    Text(a.title, color = h.text, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    Text(
                        listOf(timeRange(a), place).filter { it.isNotEmpty() }.joinToString(" · "), color = h.dim, fontSize = 12.sp,
                        maxLines = 2, overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            val link = a.onlineUrl
            if (link != null && height >= 44.dp) {
                Row(
                    Modifier.clip(RoundedCornerShape(6.dp)).background(h.accent)
                        .clickable { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(link))) }.padding(8.dp, 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(Icons.Outlined.Videocam, contentDescription = null, tint = h.onAccent, modifier = Modifier.size(16.dp))
                    Spacer(Modifier.width(4.dp))
                    Text(stringResource(R.string.join), color = h.onAccent, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
                }
            }
        }
    }
}

/** "14:00–15:00", on the phone's clock. */
fun timeRange(a: AppointmentRow): String = a.start.local().format(timeFormat) + "–" + a.end.local().format(timeFormat)

@Composable
private fun whenText(a: AppointmentRow): String {
    val date = dayTitle(a.start.local().toLocalDate())
    if (a.allDay) {
        val last = (a.end - 1).local().toLocalDate()
        return if (last != a.start.local().toLocalDate()) "$date – ${dayTitle(last)}" else "$date · ${stringResource(R.string.all_day)}"
    }
    return "$date · ${timeRange(a)}"
}

// One appointment

@Composable
private fun AppointmentScreen(store: Store, id: String, start: Long) {
    val h = LocalHelder.current
    val context = LocalContext.current
    @Suppress("UNUSED_VARIABLE") val revision = store.revision
    BackHandler { store.openAppointment = null }
    val day = start.local().toLocalDate()
    val a = store.appointments(day.minusDays(40), day.plusDays(1)).firstOrNull { it.id == id && it.start == start }
    var confirming by rememberSaveable { mutableStateOf(false) }
    Column(Modifier.fillMaxSize()) {
        TopAppBar(
            title = {},
            navigationIcon = {
                IconButton(onClick = { store.openAppointment = null }) { Icon(Icons.AutoMirrored.Outlined.ArrowBack, stringResource(R.string.back)) }
            },
            actions = {
                if (a != null) {
                    if (a.recurrence == null) {
                        IconButton(onClick = { store.editing = formFor(a) }) { Icon(Icons.Outlined.Edit, stringResource(R.string.edit)) }
                    }
                    IconButton(onClick = { confirming = true }) { Icon(Icons.Outlined.Delete, stringResource(R.string.delete)) }
                }
            },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = h.window),
        )
        if (a == null) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Text(stringResource(R.string.appointment_gone), color = h.dim) }
            return@Column
        }
        Column(Modifier.verticalScroll(rememberScrollState()).padding(16.dp, 0.dp, 16.dp, 16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Row(Modifier.height(IntrinsicSize.Min)) {
                Box(Modifier.width(5.dp).fillMaxHeight().clip(RoundedCornerShape(2.dp)).background(calendarColor(a.color)))
                Spacer(Modifier.width(10.dp))
                Column {
                    Text(a.title, style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.Bold, color = h.text, modifier = Modifier.semantics { heading() })
                    Text(whenText(a), color = h.dim)
                }
            }
            a.onlineUrl?.let { link ->
                Row(Modifier.card().padding(14.dp), verticalAlignment = Alignment.CenterVertically) {
                    Box(Modifier.size(44.dp).clip(RoundedCornerShape(10.dp)).background(h.selected), contentAlignment = Alignment.Center) {
                        Icon(Icons.Outlined.Videocam, contentDescription = null, tint = h.ink)
                    }
                    Spacer(Modifier.width(12.dp))
                    Column(Modifier.weight(1f)) {
                        Text(stringResource(R.string.online_meeting), fontWeight = FontWeight.SemiBold, color = h.text)
                        Text(Uri.parse(link).host ?: "", color = h.dim, fontSize = 14.sp)
                    }
                    Button(
                        onClick = { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(link))) },
                        colors = ButtonDefaults.buttonColors(containerColor = h.accent, contentColor = h.onAccent),
                    ) { Text(stringResource(R.string.join)) }
                }
            }
            val lines = buildList {
                if (a.location.isNotEmpty()) add(Triple(Icons.Outlined.Place, a.location, null as String?))
                add(Triple(Icons.Outlined.Notifications, reminderText(a.reminder), null))
                store.calendar(a.calendarId)?.let { add(Triple(Icons.Outlined.CalendarMonth, it.name, it.address)) }
                a.recurrence?.let { add(Triple(Icons.Outlined.Repeat, it, null)) }
            }
            Column(Modifier.card()) {
                lines.forEachIndexed { i, (icon, text, detail) ->
                    if (i > 0) HorizontalDivider(color = h.border, modifier = Modifier.padding(start = 52.dp))
                    InfoLine(icon, text, detail)
                }
            }
            if (a.attendees.isNotEmpty()) {
                Text(stringResource(R.string.attendees, a.attendees.size), color = h.dim, fontSize = 14.sp, modifier = Modifier.semantics { heading() })
                Column(Modifier.card().padding(vertical = 4.dp)) { a.attendees.forEach { AttendeeLine(it) } }
            }
            if (a.notes.isNotEmpty()) {
                SelectionContainer(Modifier.card().fillMaxWidth()) { Text(a.notes, color = h.text, modifier = Modifier.padding(14.dp)) }
            }
        }
    }
    if (confirming && a != null) {
        AlertDialog(
            onDismissRequest = { confirming = false },
            title = { Text(stringResource(R.string.delete_appointment)) },
            confirmButton = {
                if (a.recurrence != null) {
                    Column(horizontalAlignment = Alignment.End) {
                        TextButton(onClick = { confirming = false; store.deleteAppointment(a, onlyThis = true) }) { Text(stringResource(R.string.only_this_one)) }
                        TextButton(onClick = { confirming = false; store.deleteAppointment(a, onlyThis = false) }) { Text(stringResource(R.string.whole_series)) }
                    }
                } else {
                    TextButton(onClick = { confirming = false; store.deleteAppointment(a, onlyThis = false) }) { Text(stringResource(R.string.delete)) }
                }
            },
            dismissButton = { TextButton(onClick = { confirming = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}

@Composable
private fun Modifier.card(): Modifier {
    val h = LocalHelder.current
    return fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(h.card).border(1.dp, h.border, RoundedCornerShape(12.dp))
}

private fun formFor(a: AppointmentRow) = AppointmentForm(a.id, a.calendarId, a.title, a.start, a.end, a.allDay, a.location, a.notes, a.reminder)

@Composable
private fun InfoLine(icon: ImageVector, text: String, detail: String?) {
    val h = LocalHelder.current
    Row(Modifier.fillMaxWidth().padding(14.dp, 12.dp).semantics(mergeDescendants = true) {}, verticalAlignment = Alignment.Top) {
        Icon(icon, contentDescription = null, tint = h.dim, modifier = Modifier.size(24.dp))
        Spacer(Modifier.width(14.dp))
        Column {
            Text(text, color = h.text)
            if (detail != null) Text(detail, color = h.dim, fontSize = 14.sp)
        }
    }
}

/** "15 minutes before", "No reminder". */
@Composable
fun reminderText(minutes: UInt?): String = when {
    minutes == null -> stringResource(R.string.no_reminder)
    minutes == 0u -> stringResource(R.string.at_start)
    minutes == 1440u -> stringResource(R.string.one_day_before)
    minutes % 60u == 0u -> pluralStringResource(R.plurals.hours_before, (minutes / 60u).toInt(), (minutes / 60u).toInt())
    else -> pluralStringResource(R.plurals.minutes_before, minutes.toInt(), minutes.toInt())
}

@Composable
private fun AttendeeLine(p: AttendeeRow) {
    val h = LocalHelder.current
    val answer = when {
        p.organizer -> stringResource(R.string.organizer)
        else -> when (p.answer) {
            AttendeeAnswer.ACCEPTED -> stringResource(R.string.accepted)
            AttendeeAnswer.TENTATIVE -> stringResource(R.string.tentative)
            AttendeeAnswer.DECLINED -> stringResource(R.string.declined)
            AttendeeAnswer.NONE -> stringResource(R.string.no_answer)
        }
    }
    val status = if (p.optional && !p.organizer) "$answer · ${stringResource(R.string.optional)}" else answer
    val icon = when {
        p.organizer || p.answer == AttendeeAnswer.ACCEPTED -> Icons.Outlined.CheckCircle
        p.answer == AttendeeAnswer.TENTATIVE -> Icons.AutoMirrored.Outlined.HelpOutline
        p.answer == AttendeeAnswer.DECLINED -> Icons.Outlined.HighlightOff
        else -> Icons.Outlined.Schedule
    }
    Row(Modifier.fillMaxWidth().padding(14.dp, 6.dp).semantics(mergeDescendants = true) {}, verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(40.dp).clip(CircleShape).background(avatarColor(p.email)).clearAndSetSemantics {}, contentAlignment = Alignment.Center) {
            Text(initials(p.name).ifEmpty { initials(p.email) }, color = Color.White, fontSize = 15.sp)
        }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(if (p.me) stringResource(R.string.me, p.name) else p.name, color = h.text)
            Text(status, color = h.dim, fontSize = 14.sp)
        }
        Icon(icon, contentDescription = null, tint = h.dim)
    }
}

// New and changed appointments

private val reminderChoices: List<UInt?> = listOf(null, 0u, 5u, 15u, 30u, 60u, 120u, 1440u)

@Composable
private fun AppointmentEditor(store: Store, original: AppointmentForm) {
    val h = LocalHelder.current
    BackHandler { store.editing = null }
    var title by rememberSaveable { mutableStateOf(original.title) }
    var location by rememberSaveable { mutableStateOf(original.location) }
    var allDay by rememberSaveable { mutableStateOf(original.allDay) }
    var start by rememberSaveable { mutableStateOf(original.start.local()) }
    // An all-day appointment ends at the next midnight; the form shows its last day.
    var end by rememberSaveable { mutableStateOf((if (original.allDay) original.end - 1 else original.end).local()) }
    var calendarId by rememberSaveable { mutableStateOf(original.calendarId ?: store.calendars.firstOrNull()?.id ?: "") }
    var reminder by rememberSaveable { mutableStateOf(original.reminder?.toLong() ?: -1L) }
    var notes by rememberSaveable { mutableStateOf(original.notes) }
    val messages = mapOf(
        "title" to stringResource(R.string.need_title), "end" to stringResource(R.string.end_after_start),
        "recurrence" to stringResource(R.string.series_on_computer), "calendar_id" to stringResource(R.string.choose_calendar),
    )
    fun save() {
        val from = if (allDay) start.toLocalDate().atStartOfDay() else start
        val to = if (allDay) end.toLocalDate().plusDays(1).atStartOfDay() else end
        val form = AppointmentForm(
            original.id, calendarId.ifEmpty { null }, title, from.epoch(), to.epoch(), allDay, location, notes,
            if (reminder < 0) null else reminder.toUInt(),
        )
        store.saveAppointment(form)
            .onSuccess { store.editing = null }
            .onFailure { e -> store.problem = if (e is CoreException.Invalid) messages[e.field] ?: e.message else e.message }
    }
    Column(Modifier.fillMaxSize().imePadding()) {
        TopAppBar(
            title = { Text(stringResource(if (original.id == null) R.string.new_appointment else R.string.edit_appointment)) },
            navigationIcon = { IconButton(onClick = { store.editing = null }) { Icon(Icons.Outlined.Close, stringResource(R.string.cancel)) } },
            actions = { TextButton(onClick = { save() }, enabled = title.isNotBlank()) { Text(stringResource(R.string.save)) } },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = h.window),
        )
        Column(Modifier.verticalScroll(rememberScrollState()).padding(20.dp, 8.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            OutlinedTextField(title, { title = it }, label = { Text(stringResource(R.string.subject)) }, singleLine = true, modifier = Modifier.fillMaxWidth())
            OutlinedTextField(location, { location = it }, label = { Text(stringResource(R.string.location)) }, singleLine = true, modifier = Modifier.fillMaxWidth())
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Text(stringResource(R.string.all_day), color = h.text, modifier = Modifier.weight(1f))
                Switch(allDay, { allDay = it })
            }
            DateTimeRow(stringResource(R.string.start), start, allDay) { new ->
                // Moving the start takes the end along, as Outlook does.
                end = end.plusSeconds(new.epoch() - start.epoch())
                start = new
            }
            DateTimeRow(stringResource(R.string.end), end, allDay) { end = it }
            if (store.calendars.size > 1) {
                Choice(
                    stringResource(R.string.tab_calendar),
                    store.calendars.map { c -> c.id to (c.address?.let { "${c.name} ($it)" } ?: c.name) },
                    calendarId,
                ) { calendarId = it }
            }
            val reminderLabels = reminderChoices.map { (it?.toLong() ?: -1L) to reminderText(it) }
            Choice(stringResource(R.string.reminder), reminderLabels, reminder) { reminder = it }
            OutlinedTextField(notes, { notes = it }, label = { Text(stringResource(R.string.notes)) }, minLines = 3, modifier = Modifier.fillMaxWidth())
        }
    }
}

@Composable
private fun <T> Choice(label: String, options: List<Pair<T, String>>, value: T, onChange: (T) -> Unit) {
    var open by remember { mutableStateOf(false) }
    ExposedDropdownMenuBox(expanded = open, onExpandedChange = { open = it }) {
        OutlinedTextField(
            options.firstOrNull { it.first == value }?.second ?: "", {}, readOnly = true, label = { Text(label) },
            trailingIcon = { ExposedDropdownMenuDefaults.TrailingIcon(open) },
            modifier = Modifier.fillMaxWidth().menuAnchor(MenuAnchorType.PrimaryNotEditable),
        )
        ExposedDropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            options.forEach { (key, text) ->
                DropdownMenuItem(text = { Text(text) }, onClick = { onChange(key); open = false })
            }
        }
    }
}

/** A date (and, unless all day, a time) the person picks in Material's dialogs. */
@Composable
private fun DateTimeRow(label: String, value: LocalDateTime, allDay: Boolean, onChange: (LocalDateTime) -> Unit) {
    val h = LocalHelder.current
    var pickingDate by remember { mutableStateOf(false) }
    var pickingTime by remember { mutableStateOf(false) }
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(label, color = h.text, modifier = Modifier.weight(1f))
        TextButton(onClick = { pickingDate = true }) {
            Text(value.toLocalDate().format(DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM)))
        }
        if (!allDay) TextButton(onClick = { pickingTime = true }) { Text(value.toLocalTime().format(timeFormat)) }
    }
    if (pickingDate) {
        // Material's date picker counts in UTC midnights.
        val state = rememberDatePickerState(initialSelectedDateMillis = value.toLocalDate().atStartOfDay(ZoneOffset.UTC).toInstant().toEpochMilli())
        DatePickerDialog(
            onDismissRequest = { pickingDate = false },
            confirmButton = {
                TextButton(onClick = {
                    state.selectedDateMillis?.let {
                        val d = Instant.ofEpochMilli(it).atZone(ZoneOffset.UTC).toLocalDate()
                        onChange(d.atTime(value.toLocalTime()))
                    }
                    pickingDate = false
                }) { Text(stringResource(R.string.ok)) }
            },
            dismissButton = { TextButton(onClick = { pickingDate = false }) { Text(stringResource(R.string.cancel)) } },
        ) { DatePicker(state) }
    }
    if (pickingTime) {
        val state = rememberTimePickerState(value.hour, value.minute, is24Hour = android.text.format.DateFormat.is24HourFormat(LocalContext.current))
        AlertDialog(
            onDismissRequest = { pickingTime = false },
            confirmButton = {
                TextButton(onClick = { onChange(value.withHour(state.hour).withMinute(state.minute)); pickingTime = false }) { Text(stringResource(R.string.ok)) }
            },
            dismissButton = { TextButton(onClick = { pickingTime = false }) { Text(stringResource(R.string.cancel)) } },
            text = { TimePicker(state) },
        )
    }
}
