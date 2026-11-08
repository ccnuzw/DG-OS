//! Bounded host metadata only; never stores configuration bytes or credentials.
use super::{dto::Selection, fail, Domain};
use serde::{Deserialize, Serialize};
use std::{fs::File, io::{Read, Write}, path::Path};
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Binding { pub version: u8, pub id: String, pub owner: String, pub generation: String, pub domain: Domain, pub selection: Selection, pub resource: bool }
#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::{ffi::CString, os::{fd::{AsRawFd, FromRawFd}, unix::{ffi::OsStrExt, fs::{MetadataExt, OpenOptionsExt}}}, path::Component};
    unsafe extern "C" { fn openat(fd: i32, path: *const std::ffi::c_char, flags: i32, ...) -> i32; fn mkdirat(fd:i32, path:*const std::ffi::c_char, mode:u32)->i32; fn renameat(a:i32,b:*const std::ffi::c_char,c:i32,d:*const std::ffi::c_char)->i32; fn geteuid()->u32; fn acl_get_fd_np(fd:i32,kind:i32)->*mut std::ffi::c_void; fn acl_free(acl:*mut std::ffi::c_void)->i32; }
    const NOFOLLOW:i32=0x100; const DIRECTORY:i32=0x100000; const CLOEXEC:i32=0x1000000;
    fn private(file:&File)->Result<(),String> {
        let m=file.metadata().map_err(|_|fail("Unavailable"))?;
        if m.uid()!=unsafe{geteuid()} || m.mode()&0o077!=0 {return Err(fail("AccessDenied"));}
        let acl=unsafe{acl_get_fd_np(file.as_raw_fd(),0x100)};
        if acl.is_null() {if std::io::Error::last_os_error().raw_os_error()==Some(2){return Ok(());}}
        else {unsafe{acl_free(acl)};}
        Err(fail("AccessDenied"))
    }
    fn opened(fd:i32)->Result<File,String>{if fd<0{Err(fail("Unavailable"))}else{Ok(unsafe{File::from_raw_fd(fd)})}}
    pub struct Index { dir:File }
    impl Index {
        pub fn open(path:&Path,create:bool)->Result<Self,String>{
            if !path.is_absolute(){return Err(fail("UnsafePath"));}
            let mut dir=std::fs::OpenOptions::new().read(true).custom_flags(DIRECTORY|CLOEXEC).open("/").map_err(|_|fail("Unavailable"))?;
            for part in path.components(){match part{Component::RootDir=>{},Component::Normal(name)=>{
                let name=CString::new(name.as_bytes()).map_err(|_|fail("UnsafePath"))?;
                let mut fd=unsafe{openat(dir.as_raw_fd(),name.as_ptr(),DIRECTORY|NOFOLLOW|CLOEXEC)};
                if fd<0 && create && std::io::Error::last_os_error().kind()==std::io::ErrorKind::NotFound {
                    let m=dir.metadata().map_err(|_|fail("Unavailable"))?;
                    if m.uid()!=unsafe{geteuid()} || m.mode()&0o022!=0{return Err(fail("AccessDenied"));}
                    if unsafe{mkdirat(dir.as_raw_fd(),name.as_ptr(),0o700)}!=0{return Err(fail("Conflict"));}
                    dir.sync_all().map_err(|_|fail("Unavailable"))?;
                    fd=unsafe{openat(dir.as_raw_fd(),name.as_ptr(),DIRECTORY|NOFOLLOW|CLOEXEC)};
                }
                if fd<0 && std::io::Error::last_os_error().kind()==std::io::ErrorKind::NotFound{return Err(fail("NotFound"));}
                dir=opened(fd)?;
            },_=>return Err(fail("UnsafePath"))}}
            private(&dir)?;Ok(Self{dir})
        }
        pub fn load(&self,id:&str)->Result<Binding,String>{
            uuid::Uuid::parse_str(id).map_err(|_|fail("NotFound"))?;
            let name=CString::new(format!("host-{id}.json")).map_err(|_|fail("NotFound"))?;
            let fd=unsafe{openat(self.dir.as_raw_fd(),name.as_ptr(),NOFOLLOW|CLOEXEC)};
            if fd<0{return Err(fail("NotFound"));}
            let file=opened(fd)?;private(&file)?;
            let m=file.metadata().map_err(|_|fail("Unavailable"))?;
            if !m.is_file()||m.nlink()!=1||m.len()>8192{return Err(fail("RecoveryRequired"));}
            let mut bytes=Vec::new();file.take(8193).read_to_end(&mut bytes).map_err(|_|fail("Unavailable"))?;
            if bytes.len()>8192{return Err(fail("Limit"));}
            let record:Binding=serde_json::from_slice(&bytes).map_err(|_|fail("RecoveryRequired"))?;
            if record.id!=id||record.version!=1{return Err(fail("RecoveryRequired"));}Ok(record)
        }
        pub fn save(&self,record:&Binding)->Result<(),String>{
            uuid::Uuid::parse_str(&record.id).map_err(|_|fail("InvalidFormat"))?;
            let bytes=serde_json::to_vec(record).map_err(|_|fail("Unavailable"))?;
            if bytes.len()>8192{return Err(fail("Limit"));}
            let temp=CString::new(format!("host-stage-{}",uuid::Uuid::new_v4())).unwrap();
            let destination=CString::new(format!("host-{}.json",record.id)).unwrap();
            let fd=unsafe{openat(self.dir.as_raw_fd(),temp.as_ptr(),1|0x200|0x800|NOFOLLOW|CLOEXEC,0o600u32)};
            let mut file=opened(fd)?;private(&file)?;
            file.write_all(&bytes).and_then(|_|file.sync_all()).map_err(|_|fail("Unavailable"))?;
            if unsafe{renameat(self.dir.as_raw_fd(),temp.as_ptr(),self.dir.as_raw_fd(),destination.as_ptr())}!=0{return Err(fail("Unavailable"));}
            self.dir.sync_all().map_err(|_|fail("Unavailable"))
        }
    }
}
#[cfg(target_os = "macos")]
pub use platform::Index;
#[cfg(not(target_os = "macos"))]
pub struct Index;
#[cfg(not(target_os = "macos"))]
impl Index {pub fn open(_: &Path,_:bool)->Result<Self,String>{Err(fail("Unsupported"))}pub fn load(&self,_:&str)->Result<Binding,String>{Err(fail("Unsupported"))}pub fn save(&self,_:&Binding)->Result<(),String>{Err(fail("Unsupported"))}}
