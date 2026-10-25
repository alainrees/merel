@file:OptIn(ExperimentalMaterial3Api::class)

package nl.merel.app

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.PriorityHigh
import androidx.compose.material.icons.outlined.RadioButtonUnchecked
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconToggleButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import uniffi.merel_mobile.CoreException
import uniffi.merel_mobile.TaskForm
import uniffi.merel_mobile.TaskPriority
import uniffi.merel_mobile.TaskRepeat
import uniffi.merel_mobile.TaskRow

private fun Long.day(): LocalDate = Instant.ofEpochSecond(this).atZone(ZoneId.systemDefault()).toLocalDate()

/** Tasks: the open ones by due day, a tap on the circle to finish one, and a form to add or change one. */
@Composable
fun TasksTab(store: Store) {
    store.editingTask?.let {
        TaskEditor(store, it)
        return
    }
    val h = LocalHelder.current
    val today = LocalDate.now()
    val groups = listOf(
        R.string.tasks_overdue to store.tasks.filter { !it.done && it.due?.day()?.isBefore(today) == true },
        R.string.today to store.tasks.filter { !it.done && it.due?.day() == today },
        R.string.tomorrow to store.tasks.filter { !it.done && it.due?.day() == today.plusDays(1) },
        R.string.tasks_later to store.tasks.filter { !it.done && it.due?.day()?.isAfter(today.plusDays(1)) == true },
        R.string.tasks_no_date to store.tasks.filter { !it.done && it.due == null },
        R.string.tasks_done to store.tasks.filter { it.done },
    ).filter { it.second.isNotEmpty() }
    Box(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            TopAppBar(
                title = { Text(stringResource(R.string.tab_tasks)) },
                actions = {
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(end = 8.dp)) {
                        Text(stringResource(R.string.show_done_tasks), color = h.dim, style = MaterialTheme.typography.bodyMedium)
                        Checkbox(store.showDoneTasks, { store.showDone(it) })
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = h.window),
            )
            if (groups.isEmpty()) {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Text(stringResource(R.string.no_tasks), color = h.dim)
                }
            } else {
                LazyColumn(Modifier.fillMaxSize().padding(horizontal = 16.dp)) {
                    for ((title, tasks) in groups) {
                        item(key = "head-$title") {
                            Text(
                                stringResource(title), color = h.dim, style = MaterialTheme.typography.labelLarge,
                                modifier = Modifier.padding(4.dp, 16.dp, 4.dp, 6.dp).semantics { heading() },
                            )
                        }
                        items(tasks, key = { it.id }) { t ->
                            TaskLine(store, t)
                            HorizontalDivider(color = h.border)
                        }
                    }
                }
            }
        }
        FloatingActionButton(
            onClick = { store.editingTask = TaskForm(null, null, "", "", null, TaskPriority.NORMAL, null, 1u) },
            containerColor = h.accent,
            modifier = Modifier.align(Alignment.BottomEnd).padding(20.dp),
        ) { Icon(Icons.Outlined.Add, stringResource(R.string.new_task), tint = h.onAccent) }
    }
}

@Composable
private fun TaskLine(store: Store, t: TaskRow) {
    val h = LocalHelder.current
    val overdue = !t.done && t.due?.day()?.isBefore(LocalDate.now()) == true
    val details = listOfNotNull(
        t.due?.day()?.format(DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM)),
        t.repeatText,
        t.accountAddress,
        stringResource(R.string.low_priority).takeIf { t.priority == TaskPriority.LOW },
    ).joinToString(" · ")
    Row(
        Modifier.fillMaxWidth().background(h.card).clickable {
            store.editingTask = TaskForm(t.id, t.accountId, t.title, t.notes, t.due, t.priority, t.repeat, t.repeatInterval)
        }.padding(vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconToggleButton(checked = t.done, onCheckedChange = { store.completeTask(t.id, it) }) {
            Icon(
                if (t.done) Icons.Outlined.CheckCircle else Icons.Outlined.RadioButtonUnchecked,
                stringResource(if (t.done) R.string.reopen_task else R.string.complete_task),
                tint = if (t.done) h.accent else h.dim,
            )
        }
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (t.priority == TaskPriority.HIGH) {
                    Icon(Icons.Outlined.PriorityHigh, stringResource(R.string.high_priority), tint = h.flag)
                }
                Text(
                    t.title, color = if (t.done) h.dim else h.text,
                    textDecoration = if (t.done) TextDecoration.LineThrough else null,
                )
            }
            if (details.isNotEmpty()) {
                Text(details, color = if (overdue) h.flag else h.dim, style = MaterialTheme.typography.bodySmall)
            }
        }
    }
}

private val repeatChoices: List<TaskRepeat?> = listOf(null, TaskRepeat.DAILY, TaskRepeat.WEEKLY, TaskRepeat.MONTHLY, TaskRepeat.YEARLY)

@Composable
private fun TaskEditor(store: Store, original: TaskForm) {
    val h = LocalHelder.current
    BackHandler { store.editingTask = null }
    var title by rememberSaveable { mutableStateOf(original.title) }
    var notes by rememberSaveable { mutableStateOf(original.notes) }
    var hasDue by rememberSaveable { mutableStateOf(original.due != null) }
    var due by rememberSaveable { mutableStateOf((original.due?.day() ?: LocalDate.now()).atStartOfDay()) }
    var priority by rememberSaveable { mutableStateOf(original.priority) }
    var repeat by rememberSaveable { mutableStateOf(repeatChoices.indexOf(original.repeat).coerceAtLeast(0)) }
    var interval by rememberSaveable { mutableStateOf(original.repeatInterval.toInt().coerceAtLeast(1)) }
    var place by rememberSaveable { mutableStateOf(original.accountId ?: "") }
    var confirmDelete by remember { mutableStateOf(false) }
    // Only a task on this phone repeats, as on the computer.
    val canRepeat = hasDue && place.isEmpty()
    val messages = mapOf("title" to stringResource(R.string.need_task_title), "repeat" to stringResource(R.string.repeat_needs_due))
    fun save() {
        val form = TaskForm(
            original.id, place.ifEmpty { null }, title.trim(), notes,
            if (hasDue) due.toLocalDate().atStartOfDay(ZoneId.systemDefault()).toEpochSecond() else null,
            priority, if (canRepeat) repeatChoices[repeat] else null, interval.toUInt(),
        )
        store.saveTask(form)
            .onSuccess { store.editingTask = null }
            .onFailure { e -> store.problem = if (e is CoreException.Invalid) messages[e.field] ?: e.message else e.message }
    }
    Column(Modifier.fillMaxSize().imePadding()) {
        TopAppBar(
            title = { Text(stringResource(if (original.id == null) R.string.new_task else R.string.edit_task)) },
            navigationIcon = { IconButton(onClick = { store.editingTask = null }) { Icon(Icons.Outlined.Close, stringResource(R.string.cancel)) } },
            actions = {
                if (original.id != null) {
                    IconButton(onClick = { confirmDelete = true }) { Icon(Icons.Outlined.Delete, stringResource(R.string.delete_task)) }
                }
                TextButton(onClick = { save() }, enabled = title.isNotBlank()) { Text(stringResource(R.string.save)) }
            },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = h.window),
        )
        Column(Modifier.verticalScroll(rememberScrollState()).padding(20.dp, 8.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            OutlinedTextField(title, { title = it }, label = { Text(stringResource(R.string.task)) }, singleLine = true, modifier = Modifier.fillMaxWidth())
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Text(stringResource(R.string.due_date), color = h.text, modifier = Modifier.weight(1f))
                Switch(hasDue, { hasDue = it })
            }
            if (hasDue) DateTimeRow(stringResource(R.string.date), due, allDay = true) { due = it }
            Choice(
                stringResource(R.string.priority),
                listOf(TaskPriority.LOW to stringResource(R.string.priority_low), TaskPriority.NORMAL to stringResource(R.string.priority_normal), TaskPriority.HIGH to stringResource(R.string.priority_high)),
                priority,
            ) { priority = it }
            if (canRepeat) {
                val labels = listOf(R.string.repeat_never, R.string.repeat_daily, R.string.repeat_weekly, R.string.repeat_monthly, R.string.repeat_yearly)
                Choice(stringResource(R.string.repeat), labels.mapIndexed { i, l -> i to stringResource(l) }, repeat) { repeat = it }
                if (repeat > 0) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(stringResource(R.string.repeat_every, interval), color = h.text, modifier = Modifier.weight(1f))
                        TextButton(onClick = { if (interval > 1) interval-- }) { Text("−") }
                        TextButton(onClick = { if (interval < 99) interval++ }) { Text("+") }
                    }
                }
            }
            if (original.id == null && store.taskPlaces.isNotEmpty()) {
                Choice(
                    stringResource(R.string.keep_in),
                    listOf("" to stringResource(R.string.on_this_phone)) + store.taskPlaces.map { it.accountId to it.address },
                    place,
                ) { place = it }
            }
            OutlinedTextField(notes, { notes = it }, label = { Text(stringResource(R.string.notes)) }, minLines = 3, modifier = Modifier.fillMaxWidth())
            Spacer(Modifier.width(1.dp))
        }
    }
    if (confirmDelete) {
        AlertDialog(
            onDismissRequest = { confirmDelete = false },
            title = { Text(stringResource(R.string.delete_task_question)) },
            confirmButton = {
                TextButton(onClick = {
                    original.id?.let { store.deleteTask(it) }
                    confirmDelete = false
                    store.editingTask = null
                }) { Text(stringResource(R.string.delete)) }
            },
            dismissButton = { TextButton(onClick = { confirmDelete = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}
