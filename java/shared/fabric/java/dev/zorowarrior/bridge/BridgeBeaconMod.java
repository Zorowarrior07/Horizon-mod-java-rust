package dev.zorowarrior.bridge;

import net.fabricmc.api.ModInitializer;
import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.block.AbstractBlock;
import net.minecraft.block.Block;
import net.minecraft.item.BlockItem;
import net.minecraft.item.Item;
import net.minecraft.registry.Registries;
import net.minecraft.registry.Registry;
import net.minecraft.registry.RegistryKey;
import net.minecraft.registry.RegistryKeys;
import net.minecraft.sound.BlockSoundGroup;
import net.minecraft.util.Identifier;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * ZOROWARRIOR HORIZON BRIDGE — Java Edition, Fabric build.
 *
 * <p>Written in <b>Yarn</b> names, because that is the mapping set the Fabric
 * toolchain compiles and remaps against ({@code net.minecraft.block.Block},
 * {@code net.minecraft.util.Identifier}). The NeoForge builds of the same mod use
 * Mojang official names — same content, different names, which is why they are
 * separate sources and separate jars.
 *
 * <p>One detail that cost a server start and is worth writing down: since 1.21.3,
 * {@code AbstractBlock.Settings} must be handed the block's own registry key
 * ({@code registryKey(...)}) <i>before</i> the Block is constructed. Without it,
 * {@code Block}'s constructor throws {@code NullPointerException: Block id not set}
 * while computing the default loot table. Compiling does not catch this; booting
 * a server does.
 *
 * <p>Deliberately no Fabric API dependency: vanilla classes only, so Fabric
 * Loader by itself is enough to run this.
 */
public final class BridgeBeaconMod implements ModInitializer {

    public static final String MOD_ID = "zoro_bridge";
    public static final Logger LOGGER = LoggerFactory.getLogger("ZoroHorizonBridge");

    public static Block bridgeBeacon;
    public static Item bridgeChip;

    private static Identifier id(String path) {
        return Identifier.of(MOD_ID, path);
    }

    private static RegistryKey<Block> blockKey(String path) {
        return RegistryKey.of(RegistryKeys.BLOCK, id(path));
    }

    private static RegistryKey<Item> itemKey(String path) {
        return RegistryKey.of(RegistryKeys.ITEM, id(path));
    }

    @Override
    public void onInitialize() {
        Block block = new Block(AbstractBlock.Settings.create()
                .registryKey(blockKey("bridge_beacon"))
                .strength(3.5F, 6.0F)
                .luminance(state -> 12)
                .sounds(BlockSoundGroup.STONE)
                .requiresTool());

        bridgeBeacon = Registry.register(Registries.BLOCK, blockKey("bridge_beacon"), block);

        bridgeChip = Registry.register(Registries.ITEM, itemKey("bridge_chip"),
                new Item(new Item.Settings()));

        Registry.register(Registries.ITEM, itemKey("bridge_beacon"),
                new BlockItem(block, new Item.Settings()));

        String version = FabricLoader.getInstance().getModContainer(MOD_ID)
                .map(c -> c.getMetadata().getVersion().getFriendlyString())
                .orElse("unknown");
        LOGGER.info("[ZoroHorizonBridge] mod {} loaded: bridge_beacon + bridge_chip registered",
                version);
    }
}
