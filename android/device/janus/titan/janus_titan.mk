# JanusOS product for the Radxa CM5 (Pandora Titan handheld).
#
# This product does not replace the Radxa BSP: it inherits it, so the kernel,
# device tree (rk3588s-radxa-cm5-io), vendor blobs and u-boot config still come
# from the vendor tree. Everything Janus-specific is layered on top:
#
#   * JanusCore          privileged, platform-signed engine + kiosk shell
#   * no setup wizard    the device boots straight into the dashboard
#
# Build:
#   source build/envsetup.sh
#   lunch janus_titan-userdebug
#   module build:  m JanusCore
#   full image:    . build.sh -UACKu
#
# If the Radxa tree names its product differently, override the base product:
#   JANUS_BASE_PRODUCT=device/rockchip/rk3588/RadxaCM5.mk make janus_titan

JANUS_BASE_PRODUCT ?= device/rockchip/rk3588/RadxaCM5.mk

ifeq ($(wildcard $(JANUS_BASE_PRODUCT)),)
$(error JanusOS: base product "$(JANUS_BASE_PRODUCT)" not found. \
  Point JANUS_BASE_PRODUCT at the Radxa CM5 product makefile before lunching.)
endif

$(call inherit-product, $(JANUS_BASE_PRODUCT))

# Identity. PRODUCT_DEVICE is intentionally left as the vendor value so kernel
# and dtb paths (`rk3588s-radxa-cm5-io`) keep resolving.
PRODUCT_NAME := janus_titan
PRODUCT_BRAND := Janus
PRODUCT_MODEL := JanusOS Pip-Boy
PRODUCT_MANUFACTURER := Janus

# The engine, its kiosk shell, and the privileged-permission allowlist. The APK
# is a Gradle build imported as a prebuilt; see janus_core/Android.bp.
PRODUCT_PACKAGES += \
    JanusCore \
    privapp-permissions-com.janus.omega \
    default-permissions-com.janus.omega

# Appliance behaviour: no setup wizard, and a fixed baseline timezone so audit
# timestamps are comparable across devices (operators can change it later).
PRODUCT_PROPERTY_OVERRIDES += \
    ro.setupwizard.mode=DISABLED \
    persist.sys.timezone=UTC \
    ro.janus.os=1

# The privileged-permission allowlist is installed by the prebuilt_etc module in
# janus_core/Android.bp (a PRODUCT_COPY_FILES duplicate would be rejected as a
# conflicting definition).
