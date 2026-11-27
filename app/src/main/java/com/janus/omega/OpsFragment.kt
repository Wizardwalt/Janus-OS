package com.janus.omega

import android.content.Context
import android.graphics.Typeface
import android.os.Bundle
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.BaseExpandableListAdapter
import android.widget.ExpandableListView
import android.widget.TextView
import android.widget.Toast
import androidx.fragment.app.Fragment

class OpsFragment : Fragment() {

    override fun onCreateView(
        inflater: LayoutInflater, container: ViewGroup?, savedInstanceState: Bundle?
    ): View = inflater.inflate(R.layout.fragment_ops, container, false)

    override fun onViewCreated(view: View, savedInstanceState: Bundle?) {
        super.onViewCreated(view, savedInstanceState)
        view.findViewById<TextView>(R.id.catalog_header).text =
            "  MODULE CATALOG :: ${ModuleData.totalEntries} ENTRIES :: REFERENCE ONLY"
        val listView = view.findViewById<ExpandableListView>(R.id.module_list)
        val adapter = ModuleListAdapter(requireContext(), ModuleData.categories)
        listView.setAdapter(adapter)
        listView.setOnChildClickListener { _, _, groupPos, childPos, _ ->
            val module = ModuleData.categories[groupPos].second[childPos]
            Toast.makeText(
                requireContext(),
                "CATALOG ONLY: $module has no execution adapter.",
                Toast.LENGTH_LONG
            ).show()
            true
        }
    }
}

class ModuleListAdapter(
    private val context: Context,
    private val data: List<Pair<String, List<String>>>
) : BaseExpandableListAdapter() {

    override fun getGroupCount() = data.size
    override fun getChildrenCount(g: Int) = data[g].second.size
    override fun getGroup(g: Int) = data[g]
    override fun getChild(g: Int, c: Int) = data[g].second[c]
    override fun getGroupId(g: Int) = g.toLong()
    override fun getChildId(g: Int, c: Int) = c.toLong()
    override fun hasStableIds() = false
    override fun isChildSelectable(g: Int, c: Int) = true

    override fun getGroupView(g: Int, expanded: Boolean, v: View?, parent: ViewGroup?): View {
        val tv = TextView(context)
        val arrow = if (expanded) "▼" else "▶"
        val (name, modules) = data[g]
        tv.text = "  $arrow  $name  [${modules.size}]"
        tv.textSize = 13f
        tv.setTextColor(0xFF9D00FF.toInt())
        tv.typeface = Typeface.MONOSPACE
        tv.setPadding(24, 22, 24, 22)
        tv.setBackgroundColor(0xFF0F0F0F.toInt())
        return tv
    }

    override fun getChildView(g: Int, c: Int, isLast: Boolean, v: View?, parent: ViewGroup?): View {
        val tv = TextView(context)
        tv.text = "      ▸  ${data[g].second[c]}"
        tv.textSize = 12f
        tv.setTextColor(0xFF00FF41.toInt())
        tv.typeface = Typeface.MONOSPACE
        tv.setPadding(48, 14, 24, 14)
        tv.setBackgroundColor(0xFF0A0A0A.toInt())
        return tv
    }
}
