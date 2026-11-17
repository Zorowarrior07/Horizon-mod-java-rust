package dev.zorowarrior.bridge;

import net.fabricmc.api.ModInitializer;
import net.minecraft.core.Registry;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.ResourceKey;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * ZOROWARRIOR HORIZON BRIDGE — the Java Edition half.
 *
 * <p>This mod deliberately uses nothing but vanilla classes. It does not depend
 * on the Fabric API, so it installs with Fabric Loader alone and there is one
 * less moving part between "compiled" and "runs".
 *
 * <p>What it adds: a {@code zoro_bridge:bridge_beacon} block and a
 * {@code zoro_bridge:bridge_chip} item. They are the same names and the same
 * textures the Horizon {@code .hmod} ships, so a world built in one can be
 * recognised in the other — that is the whole "bridge" idea. It is a content
 * bridge, not a runtime bridge: Horizon has no JVM, so this jar cannot and does
 * not load there.
 */
public final class BridgeBeaconMod implements ModInitializer {

    public static final String MOD_ID = "zoro_bridge";
    public static final Logger LOGGER = LoggerFactory.getLogger("ZoroHorizonBridge");

    public static final ResourceLocation BEACON_ID = id("bridge_beacon");
    public static final ResourceLocation CHIP_ID = id("bridge_chip");

    /** The block instance, kept so other code (and the self-test) can reach it. */
    public static Block bridgeBeacon;

    private static ResourceLocation id(String path) {
        return ResourceLocation.fromNamespaceAndPath(MOD_ID, path);
    }

    @Override
    public void onInitialize() {
        // Note the mapping names: intermediary/official names, resolved by Loom
        // at build time. On 1.21.x the constructor is (Properties), and the
        // properties class is BlockBehaviour.Properties.
        Block block = new Block(BlockBehaviour.Properties.of()
                .strength(3.5F, 6.0F)
                .lightLevel(state -> 12)
                .sound(SoundType.STONE)
                .requiresCorrectToolForDrops());

        bridgeBeacon = register(BuiltInRegistries.BLOCK, Registries.BLOCK, "bridge_beacon", block);

        Item chip = new Item(new Item.Properties());
        register(BuiltInRegistries.ITEM, Registries.ITEM, "bridge_chip", chip);

        BlockItem blockItem = new BlockItem(block, new Item.Properties());
        register(BuiltInRegistries.ITEM, Registries.ITEM, "bridge_beacon", blockItem);

        LOGGER.info("[ZoroHorizonBridge] loaded on {} — bridge_beacon + bridge_chip registered",
                net.fabricmc.loader.api.FabricLoader.getInstance()
                        .getModContainer(MOD_ID)
                        .map(c -> c.getMetadata().getVersion().getFriendlyString())
                        .orElse("unknown"));
    }

    private static <T> T register(Registry<T> registry, ResourceKey<Registry<T>> key, String path, T value) {
        return Registry.register(registry, ResourceKey.create(key, id(path)), value);
    }
}
