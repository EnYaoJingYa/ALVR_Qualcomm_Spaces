//! Read-only QXR counter adapter, matched to the installed RVRPlayer 5.0.4 ABI.
//! Native RVAs 0x359ec and 0x35398 define the helper and the counter accessor.
use jni::{
    jni_sig, jni_str,
    objects::{JClass, JObject},
    refs::Reference,
};
use std::{ffi::c_void, ptr};

#[repr(C)]
struct Api {
    version: u32,
    padding: u32,
    functions: *const *const c_void,
}
#[repr(C)]
struct Client {
    implementation: *mut c_void,
    api: *const Api,
    context: *mut c_void,
}

pub struct VrxCounter {
    library: *mut c_void,
    client: *mut Client,
    first_count: Option<u32>,
}

impl VrxCounter {
    pub fn open() -> Option<Self> {
        if crate::fault_check("vrx_phase_service", "instance,session").is_err() {
            return None;
        }
        let android = ndk_context::android_context();
        let vm = unsafe { jni::JavaVM::from_raw(android.vm().cast()) };
        // The vendor helper loads QXRSocketFetcher through this Context's
        // ClassLoader. NativeActivity's loader does not contain that class.
        // Use installed system code only; no vendor dex or .so enters the APK.
        vm.attach_current_thread(|env| -> jni::errors::Result<Option<Self>> {
            let package = env.new_string("com.qualcomm.qti.spaces.services")?;
            let application = env.call_method(
                unsafe { JObject::global_kind_from_raw(android.context().cast()) },
                jni_str!("getApplicationContext"),
                jni_sig!("()Landroid/content/Context;"), &[],
            )?.l()?;
            let installed = env.call_method(
                &application,
                jni_str!("createPackageContext"),
                jni_sig!("(Ljava/lang/String;I)Landroid/content/Context;"),
                &[(&package).into(), 3.into()],
            )?.l()?;
            let service_loader = env.call_method(&installed, jni_str!("getClassLoader"),
                jni_sig!("()Ljava/lang/ClassLoader;"), &[])?.l()?;
            let parent = env.call_method(&application, jni_str!("getClassLoader"),
                jni_sig!("()Ljava/lang/ClassLoader;"), &[])?.l()?;
            let bytes = env.byte_array_from_slice(include_bytes!(concat!(env!("OUT_DIR"), "/classes.dex")))?;
            let buffer = env.call_static_method(jni_str!("java/nio/ByteBuffer"),
                jni_str!("wrap"), jni_sig!("([B)Ljava/nio/ByteBuffer;"), &[(&bytes).into()])?.l()?;
            let loader = env.new_object(jni_str!("dalvik/system/InMemoryDexClassLoader"),
                jni_sig!("(Ljava/nio/ByteBuffer;Ljava/lang/ClassLoader;)V"),
                &[(&buffer).into(), (&parent).into()])?;
            let name = env.new_string("alvr.client.VrxProbeContext")?;
            let class = env.call_method(&loader, jni_str!("loadClass"),
                jni_sig!("(Ljava/lang/String;)Ljava/lang/Class;"), &[(&name).into()])?.l()?;
            let class = env.cast_local::<JClass>(class)?;
            let context = env.new_object(&class,
                jni_sig!("(Landroid/content/Context;Ljava/lang/ClassLoader;)V"),
                &[(&application).into(), (&service_loader).into()])?;
            Ok(Self::open_with_context(context.as_raw().cast()))
        }).unwrap_or_else(|error| {
            alvr_common::warn!("event=vrx_counter_probe stage=installed_context code=jni_context fallback=nominal_frame_pacing detail={error}");
            None
        })
    }

    fn open_with_context(context: *mut c_void) -> Option<Self> {
        unsafe {
            let library = libc::dlopen(
                c"/system_ext/app/RVRPlayer/lib/arm64/libqxrcoreclient.so".as_ptr(),
                libc::RTLD_NOW | libc::RTLD_LOCAL,
            );
            if library.is_null() {
                return None;
            }
            let symbol = libc::dlsym(
                library,
                c"_Z20QXRCoreClient_CreateP7_JavaVMP8_jobjectPKc".as_ptr(),
            );
            if symbol.is_null() {
                libc::dlclose(library);
                return None;
            }
            let create: unsafe extern "C" fn(*mut c_void, *mut c_void, *const i8) -> *mut Client =
                std::mem::transmute(symbol);
            let android = ndk_context::android_context();
            let client = create(android.vm(), context, ptr::null());
            if client.is_null() {
                alvr_common::warn!(
                    "event=vrx_counter_probe stage=create_client code=null_client fallback=nominal_frame_pacing"
                );
                libc::dlclose(library);
                return None;
            }
            let counter = Self {
                library,
                client,
                first_count: None,
            };
            if (*client).api.is_null() {
                return None;
            }
            let api = &*(*client).api;
            alvr_common::info!("event=vrx_counter_probe stage=api version={}", api.version);
            // The installed library returns table version 8 even though its
            // factory requests version 7. Native GetVsyncCount checks >= 2;
            // versions beyond the inspected 7/8 ABI remain unsupported here.
            if !(7..=8).contains(&api.version)
                || api.functions.is_null()
                || (*api.functions.add(17)).is_null()
            {
                return None;
            }
            Some(counter)
        }
    }

    pub fn read(&mut self) -> Option<u64> {
        if crate::fault_check("vrx_phase_frozen_counter", "counter_client").is_err() {
            return Some(1);
        }
        unsafe {
            let api = &*(*self.client).api;
            let get: unsafe extern "C" fn(*mut c_void, u32, *mut *const u8) -> i32 =
                std::mem::transmute(*api.functions.add(17));
            let mut data = ptr::null();
            let result = get((*self.client).context, 0, &mut data);
            if result != 0 || data.is_null() {
                alvr_common::warn!(
                    "event=vrx_counter_probe stage=read_counter code={result} null_data={} fallback=nominal_frame_pacing",
                    data.is_null()
                );
                return None;
            }
            let raw = ptr::read_unaligned(data.add(8).cast::<u32>());
            let first = *self.first_count.get_or_insert(raw);
            // Official RVA 0x35434 returns raw - first + 1. A provider reset
            // restarts this epoch and is detected by the PC controller.
            if raw < first {
                self.first_count = Some(raw);
                return Some(1);
            }
            Some(u64::from(raw - first) + 1)
        }
    }
}

impl Drop for VrxCounter {
    fn drop(&mut self) {
        unsafe {
            let client = &*self.client;
            if let Some(api) = client.api.as_ref()
                && !api.functions.is_null()
                && !(*api.functions.add(1)).is_null()
            {
                let destroy: unsafe extern "C" fn(*mut c_void) =
                    std::mem::transmute(*api.functions.add(1));
                destroy(client.context);
            }
            if !client.implementation.is_null() {
                libc::dlclose(client.implementation);
            }
            libc::free(self.client.cast());
            libc::dlclose(self.library);
        }
    }
}
