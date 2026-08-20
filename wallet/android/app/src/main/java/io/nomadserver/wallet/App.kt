package io.nomadserver.wallet

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import io.nomadserver.wallet.watch.WatchTarget
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

@Composable
fun AppRoot(vm: MainViewModel) {
    val ui by vm.ui.collectAsState()
    if (ui.paired) {
        HomeScreen(ui, vm)
    } else {
        PairScreen(ui, vm)
    }
}

@Composable
fun PairScreen(ui: UiState, vm: MainViewModel) {
    var payload by remember { mutableStateOf("") }
    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text("Pair with your server", style = MaterialTheme.typography.headlineSmall)
        Text(
            "Open the Nomad Server page on your LAN, copy the pairing JSON, and paste it here. " +
                "QR scanning lands in the next iteration.",
            style = MaterialTheme.typography.bodyMedium,
        )
        OutlinedTextField(
            value = payload,
            onValueChange = { payload = it },
            modifier = Modifier.fillMaxWidth(),
            minLines = 4,
            label = { Text("pairing JSON") },
        )
        Button(
            onClick = { vm.pair(payload) },
            enabled = payload.isNotBlank() && !ui.busy,
        ) {
            Text(if (ui.busy) "pairing…" else "Pair")
        }
        ui.error?.let {
            Text(it, color = MaterialTheme.colorScheme.error)
        }
        Spacer(Modifier.height(24.dp))
        Text("wallet pubkey (this device):", style = MaterialTheme.typography.labelSmall)
        Text(
            ui.walletPubkey,
            style = MaterialTheme.typography.bodySmall,
            fontFamily = FontFamily.Monospace,
        )
    }
}

@Composable
fun HomeScreen(ui: UiState, vm: MainViewModel) {
    var newTarget by remember { mutableStateOf("") }
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Text("Nomad Wallet", style = MaterialTheme.typography.titleLarge)
            TextButton(onClick = { vm.unpair() }) { Text("unpair") }
        }
        Text(
            "server: ${ui.serverPubkey.take(16)}…",
            style = MaterialTheme.typography.bodySmall,
            fontFamily = FontFamily.Monospace,
        )

        Card(modifier = Modifier.fillMaxWidth()) {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                val snap = ui.snapshot
                Text(
                    if (snap == null) "— sats" else "%,d sats".format(snap.confirmedSats),
                    style = MaterialTheme.typography.headlineMedium,
                )
                if (snap != null && snap.unconfirmedSats != 0L) {
                    Text(
                        "unconfirmed: %,d sats".format(snap.unconfirmedSats),
                        style = MaterialTheme.typography.bodyMedium,
                    )
                }
                Text(
                    when {
                        ui.syncing -> "syncing…"
                        snap != null -> "synced ${formatTime(snap.syncedAt)} · watching ${snap.watching}"
                        else -> "not synced yet"
                    },
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }

        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = { vm.sync() }, enabled = !ui.syncing) { Text("Sync") }
            if (ui.notifications > 0) {
                Text(
                    "${ui.notifications} new-tx notification(s)",
                    modifier = Modifier.padding(top = 12.dp),
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }

        ui.error?.let {
            Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
        }

        OutlinedTextField(
            value = newTarget,
            onValueChange = { newTarget = it },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
            label = { Text("address or xpub/ypub/zpub") },
        )
        Button(
            onClick = {
                vm.addTarget(newTarget)
                newTarget = ""
            },
            enabled = newTarget.isNotBlank(),
        ) { Text("Add watch target") }

        LazyColumn(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            items(ui.targets) { t ->
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    Text(
                        "${if (t.kind == WatchTarget.Kind.XPUB) "xpub" else "addr"}: ${t.value.take(24)}…",
                        style = MaterialTheme.typography.bodySmall,
                        fontFamily = FontFamily.Monospace,
                        modifier = Modifier.weight(1f),
                    )
                    TextButton(onClick = { vm.removeTarget(t.value) }) { Text("remove") }
                }
            }
            item {
                Text(
                    "history",
                    style = MaterialTheme.typography.labelLarge,
                    modifier = Modifier.padding(top = 12.dp),
                )
            }
            items(ui.snapshot?.history.orEmpty()) { h ->
                Text(
                    "${h.txid.take(16)}…  height ${if (h.height == 0L) "mempool" else h.height}",
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                )
            }
        }
    }
}

private fun formatTime(epochSecs: Long): String =
    SimpleDateFormat("HH:mm:ss", Locale.US).format(Date(epochSecs * 1000))

private val WatchTarget.kindName get() = if (kind == WatchTarget.Kind.XPUB) "xpub" else "addr"
