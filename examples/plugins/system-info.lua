-- plugin-name: System Info
-- plugin-description: Live system information panel
-- plugin-type: panel
-- plugin-version: 1.3.0
-- plugin-icon: system-info-icon.png
-- plugin-keybind: open_panel = cmd+shift+i | Toggle System Info panel

function render()
    ui.panel_clear()

    local platform = session.platform()

    -- Hostname & uptime
    local hostname = session.exec("hostname")
    local uptime
    if platform == "macos" then
        uptime = session.exec("uptime | sed 's/.*up /up /' | sed 's/,.*//'")
    else
        uptime = session.exec("uptime -p 2>/dev/null || uptime")
    end

    ui.panel_heading("System")
    ui.panel_kv("Host:", hostname and hostname:gsub("%s+$", "") or "unknown")
    ui.panel_kv("Platform:", platform or "unknown")
    ui.panel_kv("Uptime:", uptime and uptime:gsub("%s+$", "") or "unknown")

    ui.panel_separator()

    -- Memory
    ui.panel_heading("Memory")
    if platform == "macos" then
        local page_size = tonumber(session.exec("sysctl -n hw.pagesize")) or 4096
        local mem_total = tonumber(session.exec("sysctl -n hw.memsize")) or 0
        local vm = session.exec("vm_stat")
        if vm and vm ~= "" then
            local pages_free = tonumber(vm:match("Pages free:%s+(%d+)")) or 0
            local pages_inactive = tonumber(vm:match("Pages inactive:%s+(%d+)")) or 0
            local pages_active = tonumber(vm:match("Pages active:%s+(%d+)")) or 0
            local pages_wired = tonumber(vm:match("Pages wired down:%s+(%d+)")) or 0

            local used = (pages_active + pages_wired) * page_size
            local total_gb = string.format("%.1f GB", mem_total / 1073741824)
            local used_gb = string.format("%.1f GB", used / 1073741824)
            local frac = mem_total > 0 and (used / mem_total) or 0
            ui.panel_progress("Memory", frac, used_gb .. " / " .. total_gb)
        else
            ui.panel_label("(not available)")
        end
    elseif platform == "linux" then
        local meminfo = session.exec("cat /proc/meminfo")
        if meminfo and meminfo ~= "" then
            local total = tonumber(meminfo:match("MemTotal:%s+(%d+)")) or 0
            local available = tonumber(meminfo:match("MemAvailable:%s+(%d+)")) or 0
            local used = total - available
            local total_gb = string.format("%.1f GB", total / 1048576)
            local used_gb = string.format("%.1f GB", used / 1048576)
            local frac = total > 0 and (used / total) or 0
            ui.panel_progress("Memory", frac, used_gb .. " / " .. total_gb)
        else
            ui.panel_label("(not available)")
        end
    else
        ui.panel_label("(unsupported platform)")
    end

    ui.panel_separator()

    -- Disk usage
    ui.panel_heading("Disk Usage")
    local disk = session.exec("df -h / | tail -1")
    if disk and disk ~= "" then
        local pct = disk:match("(%d+)%%")
        if pct then
            local frac = tonumber(pct) / 100.0
            ui.panel_progress("/ usage", frac, pct .. "%")
        end
    end

    ui.panel_separator()

    -- Load average
    ui.panel_heading("Load Average")
    local load = session.exec("cat /proc/loadavg 2>/dev/null || sysctl -n vm.loadavg 2>/dev/null")
    if load and load ~= "" then
        -- Clean up macOS format: "{ 1.23 4.56 7.89 }" → "1.23 4.56 7.89"
        load = load:gsub("[{}]", ""):gsub("^%s+", ""):gsub("%s+$", "")
        local parts = {}
        for w in load:gmatch("%S+") do
            parts[#parts + 1] = w
            if #parts >= 3 then break end
        end
        if #parts >= 3 then
            ui.panel_kv("1 min:", parts[1])
            ui.panel_kv("5 min:", parts[2])
            ui.panel_kv("15 min:", parts[3])
        else
            ui.panel_text(load)
        end
    end

    ui.panel_separator()

    -- Top processes
    ui.panel_heading("Top Processes (CPU)")
    local top
    if platform == "macos" then
        top = session.exec("ps -eo comm,%cpu -r | head -6")
    else
        top = session.exec("ps -eo comm,%cpu --sort=-%cpu | head -6")
    end
    if top and top ~= "" then
        ui.panel_text(top)
    end

    ui.panel_separator()
    ui.panel_button("refresh", "Refresh Now")
end

function on_click(id)
    if id == "refresh" then
        ui.panel_clear()
        ui.panel_label("Refreshing...")
    end
end
