# JanusOS product makefiles.
#
# Only one product is defined: the Radxa CM5 appliance image. It inherits the
# Radxa BSP product and layers the JanusCore application on top, so kernel,
# device tree and vendor blobs keep coming from the vendor tree.

PRODUCT_MAKEFILES := $(LOCAL_DIR)/janus_titan.mk

COMMON_LUNCH_CHOICES := \
    janus_titan-userdebug \
    janus_titan-user
