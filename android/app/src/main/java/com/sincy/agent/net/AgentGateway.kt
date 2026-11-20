package com.sincy.agent.net

import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.WebSocket
import okhttp3.WebSocketListener

/**
 * WSS control channel to cloud-api `/api/v1/agent/connect?token=...` (ADR-005).
 * Envelopes are the sincy-protocol JSON form (schema_version checked on both
 * sides; binary protobuf framing is layered on the same model). Large media
 * goes over the independent transfer service, never over this socket.
 */
class AgentGateway(private val baseUrl: String) {

    private val client = OkHttpClient()

    fun connect(token: String, listener: WebSocketListener): WebSocket {
        val url = baseUrl
            .replace("http://", "ws://")
            .replace("https://", "wss://") + "/agent/connect?token=" + token
        val req = Request.Builder().url(url).build()
        return client.newWebSocket(req, listener)
    }
}
