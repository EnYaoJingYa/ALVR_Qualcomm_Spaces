package com.qualcomm.qti.qxrsocketfetcher;

import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.content.ServiceConnection;
import android.os.IBinder;
import android.os.Parcel;
import android.os.ParcelFileDescriptor;
import android.os.RemoteException;
import android.util.Log;

import java.io.File;
import java.io.FileOutputStream;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.Executor;

/**
 * Minimal bridge used by the system-installed libqxrcoreclient.so.
 *
 * The native library loads this class by name and calls Connect(), GetState(),
 * GetFd(), and Disconnect(). This implementation only connects to the system
 * QXRCoreService and obtains its QVR socket; it does not include any vendor
 * binary or vendor protocol code.
 */
public final class QXRSocketFetcher {
    private static final String TAG = "ALVR-QXRSocketFetcher";
    private static final String QXR_CORE_DESCRIPTOR =
            "com.qualcomm.qti.qxrcoreservice.IQXRCoreService";
    private static final String XRCB_DESCRIPTOR =
            "com.qualcomm.qti.xrcb.IXRCBService";

    private static final int STATE_READY = 0;
    private static final int STATE_PENDING = 1;
    private static final int STATE_OBTAINED = 2;
    private static final int STATE_ERROR = 3;
    private static final int STATE_SERVICE_UNAVAILABLE = 4;

    private final Context context;
    private final String serviceClass;
    private final int protocol;

    private volatile int state = STATE_READY;
    private volatile boolean bound;
    private ParcelFileDescriptor fd;

    private final ServiceConnection connection = new ServiceConnection() {
        @Override
        public void onServiceConnected(ComponentName name, IBinder service) {
            try {
                fd = getCoreServiceFd(service, protocol, isXrcbService(serviceClass));
                state = fd != null ? STATE_OBTAINED : STATE_ERROR;
                recordDiagnostic(
                        "socket-fetcher.log",
                        "onServiceConnected " + name + " state=" + state
                                + (fd != null ? " fd=" + GetFd() : " fd=null") + "\n"
                );
            } catch (Throwable error) {
                Log.e(TAG, "QXRCoreService transaction failed", error);
                state = STATE_ERROR;
                recordDiagnostic("socket-fetcher.log", "onServiceConnected failed " + error + "\n");
            }
        }

        @Override
        public void onServiceDisconnected(ComponentName name) {
            state = STATE_SERVICE_UNAVAILABLE;
            recordDiagnostic("socket-fetcher.log", "onServiceDisconnected " + name + "\n");
        }
    };

    public QXRSocketFetcher(Context context, String serviceClass, int protocol) {
        this.context = context.getApplicationContext();
        this.serviceClass = serviceClass;
        this.protocol = protocol;
        Log.d(TAG, "requested service=" + serviceClass + " protocol=" + protocol);
        recordDiagnostic(
                "socket-fetcher.log",
                "requested service=" + serviceClass + " protocol=" + protocol + "\n"
        );
    }

    public void Connect() {
        state = STATE_PENDING;
        int separator = serviceClass.contains("/")
                ? serviceClass.indexOf('/')
                : serviceClass.lastIndexOf('.');
        String packageName = serviceClass.substring(0, separator);
        String className = serviceClass.substring(separator + 1);
        if (!className.contains(".")) {
            className = packageName + "." + className;
        }

        Intent intent = new Intent();
        intent.setClassName(packageName, className);
        intent.setAction(isXrcbService(serviceClass) ? XRCB_DESCRIPTOR : QXR_CORE_DESCRIPTOR);
        try {
            context.getPackageManager().getPackageInfo(
                    packageName,
                    PackageManager.GET_SERVICES
            );
            Log.d(TAG, "package visible: " + packageName);
        } catch (PackageManager.NameNotFoundException error) {
            Log.e(TAG, "package not visible: " + packageName);
        }

        try {
            Executor executor = Runnable::run;
            bound = context.bindService(intent, Context.BIND_AUTO_CREATE, executor, connection);
        } catch (SecurityException error) {
            Log.e(TAG, "QXRCoreService bind was rejected", error);
            bound = false;
        } catch (NoSuchMethodError error) {
            bound = context.bindService(intent, connection, Context.BIND_AUTO_CREATE);
        }

        if (!bound) {
            Log.e(TAG, "bindService failed for " + packageName + "/" + className);
            state = STATE_SERVICE_UNAVAILABLE;
        }
        recordDiagnostic(
                "socket-fetcher.log",
                "Connect package=" + packageName + " class=" + className
                        + " bound=" + bound + " state=" + state + "\n"
        );
    }

    public int GetState() {
        return state;
    }

    public int GetFd() {
        if (fd == null) {
            return -1;
        }
        try {
            return fd.getFd();
        } catch (IllegalStateException error) {
            return -1;
        }
    }

    public void Disconnect() {
        if (bound) {
            try {
                context.unbindService(connection);
            } catch (IllegalArgumentException ignored) {
            }
            bound = false;
        }
        if (fd != null) {
            try {
                fd.close();
            } catch (Exception ignored) {
            }
            fd = null;
        }
        state = STATE_READY;
    }

    private static ParcelFileDescriptor getCoreServiceFd(
            IBinder service,
            int protocol,
            boolean xrcb
    )
            throws RemoteException {
        return getServiceFd(service, protocol, xrcb);
    }

    private static boolean isXrcbService(String serviceClass) {
        return serviceClass.contains(".xrcb.");
    }

    private static ParcelFileDescriptor getServiceFd(
            IBinder service,
            int protocol,
            boolean xrcb
    ) throws RemoteException {
        Parcel data = Parcel.obtain();
        Parcel reply = Parcel.obtain();
        try {
            data.writeInterfaceToken(xrcb ? XRCB_DESCRIPTOR : QXR_CORE_DESCRIPTOR);
            data.writeInt(protocol);
            service.transact(1, data, reply, 0);
            reply.readException();

            if (reply.readInt() == 0) {
                return null;
            }
            int parcelableSize = reply.readInt();
            int start = reply.dataPosition();
            int fdValid = reply.readInt();
            ParcelFileDescriptor result = reply.readInt() != 0
                    ? ParcelFileDescriptor.CREATOR.createFromParcel(reply)
                    : null;
            reply.setDataPosition(start + parcelableSize);
            return fdValid != 0 ? result : null;
        } finally {
            reply.recycle();
            data.recycle();
        }
    }

    private void recordDiagnostic(String fileName, String message) {
        File directory = context.getExternalFilesDir(null);
        if (directory == null) {
            return;
        }
        directory.mkdirs();
        File file = new File(directory, fileName);
        try (FileOutputStream output = new FileOutputStream(file, true)) {
            output.write(message.getBytes(StandardCharsets.UTF_8));
            output.flush();
        } catch (Exception ignored) {
        }
    }
}
