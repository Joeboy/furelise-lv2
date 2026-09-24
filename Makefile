PROFILE ?= release
TARGET_DIR ?= target
BUILD_DIR ?= build
BUNDLE := $(BUILD_DIR)/furelise.lv2

ifeq ($(OS),Windows_NT)
LIBRARY := furelise_lv2.dll
else ifeq ($(shell uname -s),Darwin)
LIBRARY := libfurelise_lv2.dylib
else
LIBRARY := libfurelise_lv2.so
endif

.PHONY: bundle clean

bundle:
	cargo build --profile $(PROFILE)
	mkdir -p $(BUNDLE)
	sed 's/@BINARY@/$(LIBRARY)/' furelise.lv2/manifest.ttl > $(BUNDLE)/manifest.ttl
	cp furelise.lv2/furelise.ttl $(BUNDLE)/furelise.ttl
	cp furelise.lv2/FurElise.MID $(BUNDLE)/FurElise.MID
	cp $(TARGET_DIR)/$(PROFILE)/$(LIBRARY) $(BUNDLE)/$(LIBRARY)

clean:
	cargo clean
	rm -rf $(BUILD_DIR)
