package io.nomadserver.wallet

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import io.nomadserver.wallet.watch.BalanceSnapshot
import io.nomadserver.wallet.watch.WalletRepository
import io.nomadserver.wallet.watch.WatchTarget
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject

data class UiState(
    val paired: Boolean = false,
    val serverPubkey: String = "",
    val walletPubkey: String = "",
    val targets: List<WatchTarget> = emptyList(),
    val snapshot: BalanceSnapshot? = null,
    val syncing: Boolean = false,
    val busy: Boolean = false,
    val error: String? = null,
    val notifications: Int = 0,
)

class MainViewModel(app: Application) : AndroidViewModel(app) {

    private val repo = WalletRepository(app)

    private val _ui = MutableStateFlow(
        UiState(
            paired = repo.pairedServer() != null,
            serverPubkey = repo.pairedServer()?.nodePubkey ?: "",
            walletPubkey = repo.walletPubkey,
            targets = repo.targets(),
        )
    )
    val ui: StateFlow<UiState> = _ui

    init {
        // surface server notifications as a counter (full UX later)
        viewModelScope.launch {
            repo.notifications()?.collect { payload: JsonObject ->
                _ui.value = _ui.value.copy(notifications = _ui.value.notifications + 1)
            }
        }
    }

    fun pair(rawPayload: String) {
        viewModelScope.launch {
            _ui.value = _ui.value.copy(busy = true, error = null)
            try {
                repo.pair(rawPayload)
                _ui.value = _ui.value.copy(
                    busy = false,
                    paired = true,
                    serverPubkey = repo.pairedServer()?.nodePubkey ?: "",
                )
                sync()
            } catch (e: Exception) {
                _ui.value = _ui.value.copy(busy = false, error = e.message ?: "pairing failed")
            }
        }
    }

    fun unpair() {
        viewModelScope.launch {
            _ui.value = _ui.value.copy(busy = true)
            repo.unpair()
            _ui.value = UiState(walletPubkey = repo.walletPubkey)
        }
    }

    fun addTarget(input: String) {
        try {
            repo.addTarget(input)
            _ui.value = _ui.value.copy(targets = repo.targets(), error = null)
            sync()
        } catch (e: Exception) {
            _ui.value = _ui.value.copy(error = e.message ?: "invalid target")
        }
    }

    fun removeTarget(value: String) {
        repo.removeTarget(value)
        _ui.value = _ui.value.copy(targets = repo.targets())
        sync()
    }

    fun sync() {
        if (_ui.value.syncing || repo.pairedServer() == null) return
        viewModelScope.launch {
            _ui.value = _ui.value.copy(syncing = true, error = null)
            try {
                val snap = repo.sync()
                _ui.value = _ui.value.copy(syncing = false, snapshot = snap)
            } catch (e: Exception) {
                _ui.value = _ui.value.copy(syncing = false, error = e.message)
            }
        }
    }

    fun clearError() {
        _ui.value = _ui.value.copy(error = null)
    }
}
