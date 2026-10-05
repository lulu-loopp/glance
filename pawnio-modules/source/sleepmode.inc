#include <pawnio.inc>

const SleepMode: {
    SleepMode_AlwaysBusy = 0,
    SleepMode_ShortBusy = 1,
    SleepMode_AlwaysSleep = 2,

    SleepMode_Max,
};

static SleepMode:g_sleep_mode = SleepMode_AlwaysBusy;

/// Set sleep mode.
/// @param mode Sleep mode to set
stock Void:set_sleep_mode(SleepMode:mode) {
    g_sleep_mode = mode;
}

/// Short sleep for a given number of microseconds.
/// @param us Microseconds to sleep
stock Void:microsleep_short(us) {
    switch (g_sleep_mode) {
        case SleepMode_AlwaysBusy, SleepMode_ShortBusy:
            microsleep2(us);
        case SleepMode_AlwaysSleep:
            microsleep(us);
    }
}

/// Long sleep for a given number of microseconds.
/// @param us Microseconds to sleep
stock Void:microsleep_long(us) {
    switch (g_sleep_mode) {
        case SleepMode_AlwaysBusy:
            microsleep2(us);
        case SleepMode_ShortBusy, SleepMode_AlwaysSleep:
            microsleep(us);
    }
}

#if !defined(NO_SLEEPMODE_IOCTL)

DEFINE_IOCTL_SIZED(ioctl_set_sleep_mode, 1, 0) {
    new SleepMode:mode = SleepMode:in[0];
    if (mode < SleepMode_AlwaysBusy || mode >= SleepMode_Max) {
        return STATUS_INVALID_PARAMETER;
    }
    set_sleep_mode(mode);
    return STATUS_SUCCESS;
}

#endif
