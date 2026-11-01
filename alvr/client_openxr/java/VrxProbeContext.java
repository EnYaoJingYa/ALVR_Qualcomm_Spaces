package alvr.client;

import android.content.Context;
import android.content.ContextWrapper;

/** Loads installed service classes while retaining ALVR's caller identity. */
public final class VrxProbeContext extends ContextWrapper {
    private final ClassLoader serviceLoader;

    public VrxProbeContext(Context application, ClassLoader serviceLoader) {
        super(application);
        this.serviceLoader = serviceLoader;
    }

    @Override public ClassLoader getClassLoader() { return serviceLoader; }
    @Override public Context getApplicationContext() { return this; }
}
