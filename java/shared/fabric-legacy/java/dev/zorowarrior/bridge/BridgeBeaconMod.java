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
 * ZOROWARRIOR HORIZON BRIDGE — Fabric, Minecraft 1.21.1 and older 1.21.x.
 *
 * <p>This is the "registry key is optional" variant. In 1.21.1
 * {@code AbstractBlock.Settings} takes no registry key, and the block's id is
 * assigned by {@code Registry.register} — which is what the 1.21.1 the jar is
 * built for does. The newer 1.21.x builds use the sibling source in
 * {@code java/shared/fabric/}, because 1.21.3+ made the key mandatory and also
 * made late registration of a keyed block illegal. Two sources, because the API
 * really did change; one jar each, because of it.
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
        Block block = new Block(AbstractBlock.Settings.create()
                .strength(3.5F, 6.0F)
                .luminance(state -> 12)
                .sounds(BlockSoundGroup.STONE)
                .requiresTool());

        bridgeBeacon = Registry.register(Registries.BLOCK, id("bridge_beacon"), block);
        bridgeChip = Registry.register(Registries.ITEM, id("bridge_chip"),
                new Item(new Item.Settings()));
        Registry.register(Registries.ITEM, id("bridge_beacon"),
                new BlockItem(block, new Item.Settings()));

        String version = FabricLoader.getInstance().getModContainer(MOD_ID)
                .map(c -> c.getMetadata().getVersion().getFriendlyString())
                .orElse("unknown");
        LOGGER.info("[ZoroHorizonBridge] mod {} loaded: bridge_beacon + bridge_chip registered",
                version);
    }
}
