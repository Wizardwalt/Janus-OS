janus.log(">>> [MOD] UNIVERSAL GAME GOD")
janus.log("Injecting Infinite Coins (XML)...")
janus.shell("find /data/data -name '*.xml' -exec sed -i 's/\"coins\" value=\"[0-9]*\"/\"coins\" value=\"9999999\"/g' {} \\;")
