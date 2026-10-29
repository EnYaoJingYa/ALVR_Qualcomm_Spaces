package com.qualcomm.qti.qxrsocketfetcher;

import android.app.NativeActivity;
import android.os.Bundle;
import android.util.Log;

import java.io.File;
import java.io.FileOutputStream;
import java.io.PrintWriter;
import java.io.StringWriter;
import java.nio.charset.StandardCharsets;

public final class AlvrVrxActivity extends NativeActivity {
    private static final String TAG = "ALVR-VrxActivity";

    static {
        System.loadLibrary("alvr_client_openxr");
    }

    private static native void nativeVrxProbeInit();

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        recordDiagnostic("activity.log", "onCreate: starting native probe\n");
        Log.i(TAG, "onCreate: starting QXR low-latency probe");
        try {
            nativeVrxProbeInit();
            recordDiagnostic("activity.log", "onCreate: native probe returned\n");
            Log.i(TAG, "onCreate: QXR low-latency probe returned");
        } catch (Throwable error) {
            StringWriter trace = new StringWriter();
            error.printStackTrace(new PrintWriter(trace));
            recordDiagnostic("activity.log", "onCreate: native probe failed\n" + trace);
            Log.e(TAG, "onCreate: QXR low-latency probe failed", error);
            throw error;
        }
    }

    private void recordDiagnostic(String fileName, String message) {
        File directory = getExternalFilesDir(null);
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
