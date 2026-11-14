package com.y7ya.notes.ui

import android.text.format.DateUtils
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning
import com.y7ya.notes.data.nowSecs
import com.y7ya.notes.ui.components.AlertButton
import com.y7ya.notes.ui.components.AlertButtonStyle
import com.y7ya.notes.ui.components.BackButton
import com.y7ya.notes.ui.components.BarButton
import com.y7ya.notes.ui.components.GroupPosition
import com.y7ya.notes.ui.components.GroupedRow
import com.y7ya.notes.ui.components.IosAlert
import com.y7ya.notes.ui.components.IosIcons
import com.y7ya.notes.ui.components.IosTopBar
import com.y7ya.notes.ui.components.LargeTitle
import com.y7ya.notes.ui.components.SectionFooter
import com.y7ya.notes.ui.components.rememberTitleCollapsed
import com.y7ya.notes.ui.theme.IosType
import com.y7ya.notes.ui.theme.NotesTheme
import kotlinx.coroutines.delay
import uniffi.notes_ffi.PeerRecord

@Composable
fun SyncScreen(viewModel: SyncViewModel, onBack: () -> Unit) {
    val peers by viewModel.peers.collectAsStateWithLifecycle()
    val status by viewModel.status.collectAsStateWithLifecycle()
    val colors = NotesTheme.colors
    val context = LocalContext.current
    val focusManager = LocalFocusManager.current
    var code by rememberSaveable { mutableStateOf("") }
    var unpairing by remember { mutableStateOf<PeerRecord?>(null) }
    val pairing = status == PairStatus.Pairing

    // Ticks so "Last synced 2 minutes ago" stays current.
    val now by produceState(nowSecs()) {
        while (true) {
            delay(30_000)
            value = nowSecs()
        }
    }

    val listState = rememberLazyListState()
    val collapsed by listState.rememberTitleCollapsed()
    val top = WindowInsets.statusBars.asPaddingValues().calculateTopPadding()
    val bottom = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()

    fun scan() {
        val options = GmsBarcodeScannerOptions.Builder()
            .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
            .build()
        GmsBarcodeScanning.getClient(context, options).startScan()
            .addOnSuccessListener { barcode -> barcode.rawValue?.let(viewModel::pair) }
            .addOnFailureListener { e -> viewModel.scanFailed(e.message ?: "The code scanner couldn’t start.") }
    }

    fun pairManually() {
        focusManager.clearFocus()
        viewModel.pair(code)
    }

    Box(Modifier.fillMaxSize().background(colors.background)) {
        LazyColumn(
            state = listState,
            contentPadding = PaddingValues(top = top + 44.dp, bottom = bottom + 32.dp),
            modifier = Modifier.fillMaxSize(),
        ) {
            item(key = "title") { LargeTitle("Sync") }

            item(key = "intro") {
                Column(
                    Modifier.fillMaxWidth().padding(horizontal = 32.dp, vertical = 12.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Box(
                        Modifier.size(64.dp).clip(CircleShape).background(colors.accent),
                        contentAlignment = Alignment.Center,
                    ) {
                        Icon(IosIcons.Computer, contentDescription = null, tint = colors.card, modifier = Modifier.size(38.dp))
                    }
                    Spacer(Modifier.height(14.dp))
                    Text(
                        "Notes syncs directly between your phone and computer. No account or cloud. " +
                            "Your notes are end-to-end encrypted.",
                        style = IosType.subhead,
                        color = colors.label,
                        textAlign = TextAlign.Center,
                    )
                }
            }

            item(key = "scan") {
                GroupedRow(
                    position = GroupPosition.Single,
                    modifier = Modifier.padding(top = 12.dp),
                    onClick = if (pairing) null else ::scan,
                    leading = {
                        Icon(IosIcons.Scan, contentDescription = null, tint = colors.accent, modifier = Modifier.size(26.dp))
                    },
                ) {
                    Text(
                        "Scan Code on Computer",
                        style = IosType.body,
                        color = if (pairing) colors.secondaryLabel else colors.accent,
                    )
                }
            }
            item(key = "scan-footer") {
                SectionFooter("On your computer, open Notes and click Sync with Phone (bottom left) to show a pairing code.")
            }

            item(key = "manual-header") { GroupHeader("Enter Code Manually") }
            item(key = "manual") {
                GroupedRow(position = GroupPosition.Single, trailing = {
                    BarButton(onClick = ::pairManually, enabled = code.isNotBlank() && !pairing) {
                        Text("Pair", style = IosType.headline, color = colors.accent)
                    }
                }) {
                    Box(Modifier.weight(1f), contentAlignment = Alignment.CenterStart) {
                        if (code.isEmpty()) {
                            Text("Paste the pairing code", style = IosType.body, color = colors.secondaryLabel, maxLines = 1)
                        }
                        BasicTextField(
                            value = code,
                            onValueChange = { code = it },
                            singleLine = true,
                            enabled = !pairing,
                            textStyle = IosType.body.copy(color = colors.label),
                            cursorBrush = SolidColor(colors.accent),
                            keyboardOptions = KeyboardOptions(autoCorrectEnabled = false, imeAction = ImeAction.Go),
                            keyboardActions = KeyboardActions(onGo = { if (code.isNotBlank()) pairManually() }),
                            modifier = Modifier.fillMaxWidth(),
                        )
                    }
                }
            }

            item(key = "status") { PairStatusView(status) }

            if (peers.isNotEmpty()) {
                item(key = "peers-header") { GroupHeader("Paired Computers") }
                itemsIndexed(peers, key = { _, p -> p.id }) { i, peer ->
                    GroupedRow(
                        position = GroupPosition.of(i, peers.size),
                        modifier = Modifier.animateItem(),
                        leading = {
                            Icon(IosIcons.Computer, contentDescription = null, tint = colors.accent, modifier = Modifier.size(26.dp))
                        },
                        trailing = {
                            BarButton(onClick = { unpairing = peer }) {
                                Text("Unpair", style = IosType.body, color = colors.red)
                            }
                        },
                    ) {
                        Column(Modifier.weight(1f)) {
                            Text(peer.name, style = IosType.body, color = colors.label, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text(
                                if (peer.connected) "Connected" else lastSynced(peer.lastSync, now),
                                style = IosType.subhead,
                                color = if (peer.connected) colors.green else colors.secondaryLabel,
                            )
                        }
                    }
                }
            }
        }

        IosTopBar(
            title = "Sync",
            collapsed = collapsed,
            navigation = { BackButton("Folders", onBack) },
        )
    }

    unpairing?.let { peer ->
        IosAlert(
            title = "Unpair “${peer.name}”?",
            message = "Notes will stop syncing with this computer. Notes already on this phone stay here.",
            onDismiss = { unpairing = null },
            buttons = listOf(
                AlertButton("Cancel", AlertButtonStyle.Cancel) { unpairing = null },
                AlertButton("Unpair", AlertButtonStyle.Destructive) {
                    viewModel.unpair(peer.id)
                    unpairing = null
                },
            ),
        )
    }
}

@Composable
private fun PairStatusView(status: PairStatus) {
    val colors = NotesTheme.colors
    val modifier = Modifier.fillMaxWidth().padding(horizontal = 32.dp, vertical = 10.dp)
    when (status) {
        PairStatus.Idle -> Spacer(Modifier.height(8.dp))
        PairStatus.Pairing -> Row(modifier, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.Center) {
            CircularProgressIndicator(Modifier.size(18.dp), color = colors.secondaryLabel, strokeWidth = 2.dp)
            Spacer(Modifier.width(10.dp))
            Text("Pairing… Keep Notes open on your computer.", style = IosType.footnote, color = colors.secondaryLabel)
        }
        is PairStatus.Paired -> Text(
            "Paired with ${status.name}",
            style = IosType.subhead,
            color = colors.green,
            textAlign = TextAlign.Center,
            modifier = modifier,
        )
        is PairStatus.Failed -> Text(
            status.message,
            style = IosType.subhead,
            color = colors.red,
            textAlign = TextAlign.Center,
            modifier = modifier,
        )
    }
}

/** A Settings-style group header: small gray caps above a card. */
@Composable
private fun GroupHeader(text: String) {
    Text(
        text.uppercase(),
        style = IosType.footnote,
        color = NotesTheme.colors.secondaryLabel,
        modifier = Modifier.fillMaxWidth().padding(start = 32.dp, end = 32.dp, top = 24.dp, bottom = 7.dp),
    )
}

private fun lastSynced(lastSync: Long?, now: Long): String = when {
    lastSync == null -> "Not synced yet"
    now - lastSync < 60 -> "Last synced just now"
    else -> "Last synced " + DateUtils.getRelativeTimeSpanString(
        lastSync * 1000,
        now * 1000,
        DateUtils.MINUTE_IN_MILLIS,
        DateUtils.FORMAT_ABBREV_RELATIVE,
    )
}
