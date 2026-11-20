package com.janus.omega

import android.content.Intent
import android.os.Bundle
import android.widget.Button
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import kotlin.concurrent.thread

class HomeActivity : AppCompatActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_home)

        val statusText = findViewById<TextView>(R.id.statusText)
        val operatorButton = findViewById<Button>(R.id.operatorButton)
        val modulesButton = findViewById<Button>(R.id.modulesButton)
        val assistantButton = findViewById<Button>(R.id.assistantButton)
        val diagnosticsButton = findViewById<Button>(R.id.diagnosticsButton)
        val notesButton = findViewById<Button>(R.id.notesButton)
        val evidenceButton = findViewById<Button>(R.id.evidenceButton)
        val settingsButton = findViewById<Button>(R.id.settingsButton)

        startService(Intent(this, JanusForegroundService::class.java))

        val fullInterfaceButton = findViewById<Button>(R.id.fullInterfaceButton)
        fullInterfaceButton.setOnClickListener {
            startActivity(Intent(this, JanusWebActivity::class.java))
        }

        operatorButton.setOnClickListener {
            startActivity(Intent(this, OperatorActivity::class.java))
        }

        modulesButton.setOnClickListener {
            startActivity(Intent(this, ModulesActivity::class.java))
        }

        assistantButton.setOnClickListener {
            startActivity(Intent(this, ChatActivity::class.java))
        }

        diagnosticsButton.setOnClickListener {
            startActivity(Intent(this, DiagnosticsActivity::class.java))
        }

        notesButton.setOnClickListener {
            startActivity(Intent(this, NotesActivity::class.java))
        }

        evidenceButton.setOnClickListener {
            startActivity(Intent(this, EvidenceActivity::class.java))
        }

        settingsButton.setOnClickListener {
            startActivity(Intent(this, SettingsActivity::class.java))
        }

        thread {
            val result = RuntimeClient.get("/api/status")
            runOnUiThread {
                statusText.text = "Runtime status:\n$result"
            }
        }
    }
}
