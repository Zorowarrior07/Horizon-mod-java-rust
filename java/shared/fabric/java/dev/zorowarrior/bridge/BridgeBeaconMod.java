package dev.zorowarrior.bridge;

import net.fabricmc.api.ModInitializer;
import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.block.AbstractBlock;
import net.minecraft.block.Block;
import net.minecraft.item.BlockItem;
import net.minecraft.item.Item;
import net.minecraft.registry.Registries;
import net.minecraft.registry.Registry;
import net.minecraft.sound.BlockSoundGroup;
import net.minecraft.util.Identifier;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * ZOROWARRIOR HORIZON BRIDGE — Java Edition, Fabric build.
 *
 * <p>This file is written in <b>Yarn</b> names, because that is the mapping set
 * the Fabric toolchain compiles and remaps against (`net.minecraft.block.Block`,
 * `net.minecraft.util.Identifier`). The NeoForge and Forge builds of the same
 * mod use Mojang official names instead — same content, different names, which
 * is why they are separate sources and separate jars.
 *
 * <p>Content added: the {@code zoro_bridge:bridge_beacon} block and the
 * {@code zoro_bridge:bridge_chip} item. Same ids, same textures as the Horizon
 * {@code .hmod}, so a build made for the Horizon launcher and a build made for a
 * Java launcher describe the same objects.
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

    @Override
    public void onInitialize() {
        AbstractBlock.Settings blockSettings = AbstractBlock.Settings.create()
                .strength(3.5F, 6.0F)
                .luminance(state -> 12)
                .sounds(BlockSoundGroup.STONE)
                .requiresTool();

        bridgeBeacon = Registry.register(Registries.BLOCK, id("bridge_beacon"),
                new Block(blockSettings));

        bridgeChip = Registry.register(Registries.ITEM, id("bridge_chip"),
                new Item(new Item.Settings()));

        Registry.register(Registries.ITEM, id("bridge_beacon"),
                new BlockItem(bridgeBeacon, new Item.Settings()));

        String version = FabricLoader.getInstance().getModContainer(MOD_ID)
                .map(c -> c.getMetadata().getVersion().getFriendlyString())
                .orElse("unknown");
        LOGGER.info("[ZoroHorizonBridge] mod {} loaded: bridge_beacon + bridge_chip registered", version);
    }
}
