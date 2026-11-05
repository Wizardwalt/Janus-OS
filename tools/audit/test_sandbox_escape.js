// Verify the sandbox hardening: load full Lua (like mlua ALL_SAFE) then apply
// the same removals as new_sandbox_lua(), and confirm escapes are blocked.
const {LuaFactory}=require('wasmoon');
(async()=>{
 const f=new LuaFactory();const L=await f.createEngine();
 await L.doString(`
   io=nil; os=nil; package=nil; require=nil; module=nil; dofile=nil; loadfile=nil; debug=nil
 `);
 const attacks=[
  ['read arbitrary file','local fh=io.open("/etc/passwd","r"); return fh:read("a")'],
  ['run shell command','return os.execute("id")'],
  ['delete evidence','return os.remove("/tmp/victim")'],
  ['require os via package','return require("os")'],
  ['dofile escape','return dofile("/etc/passwd")'],
  ['loadfile escape','return loadfile("/etc/passwd")'],
  ['debug library escape','return debug.getinfo(1)'],
  ['write file via io','io.open("/tmp/pwned","w"):write("x")'],
 ];
 let blocked=0;
 for(const [name,code] of attacks){
   const r=await L.doString('local ok,err=pcall(function() '+code+' end); if ok then return "ALLOWED" else return "BLOCKED" end');
   if(r==='BLOCKED')blocked++;
   console.log('  '+(r==='BLOCKED'?'PASS':'FAIL')+'  '+name+' -> '+r);
 }
 // legitimate computation must still work
 const ok=await L.doString('return string.format("%s-%d", string.upper("ab"), math.floor(3.7))');
 console.log('  '+(ok==='AB-3'?'PASS':'FAIL')+'  legitimate string/math still work -> '+ok);
 L.global.close();
 console.log('\n'+blocked+'/'+attacks.length+' escape attempts blocked');
 process.exit(blocked===attacks.length && ok==='AB-3'?0:1);
})();
