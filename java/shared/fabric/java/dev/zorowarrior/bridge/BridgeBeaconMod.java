package dev.zorowarrior.bridge;

import net.fabricmc.api.ModInitializer;
import net.minecraft.block.AbstractBlock;
import net.minecraft.block.Block;
import net.minecraft.item.BlockItem;
import net.minecraft.item.Item;
import net.minecraft.registry.Registries;
import net.minecraft.registry.Registry;
import net.minecraft.util.Identifier;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * ZOROWARRIOR HORIZON BRIDGE — the Java Edition half, for Fabric with Yarn names.
 *
 * <p>This is a real, installable mod: it registers one block and one item, with
 * models, textures, two recipes, tags and language entries, and it says so on
 * load. It doubles as the honest counterpart to the bridge: this jar is the
 * input the Rust tool inspects, so every claim the tool makes about a Java mod
 * is tested against a mod that really exists.
 *
 * <p>Deliberately no Fabric API: registration goes through vanilla's own
 * {@code Registry}, so the jar needs nothing but the loader. That keeps one
 * dependency out of the way of both the loader and the report.
 *
 * <p>Everything here is server-safe. Rendering is data-only — blockstate,
 * models, textures — which is exactly what makes the content portable to
 * Horizon at all, and what the bridge's report is measured against.
 */
public final class BridgeBeaconMod implements ModInitializer {
    public static final String MOD_ID = "zoro_bridge";
    public static final Logger LOGGER = LoggerFactory.getLogger(MOD_ID);

    /** The block: stone-like hardness, tougher than stone, and it glows. */
    public static final Block BRIDGE_BEACON =
            new Block(AbstractBlock.Settings.create()
                    .strength(3.5f, 6.0f)
                    .luminance(state -> 12)
                    .requiresTool());

    /** The item a player crafts the beacon from. */
    public static final Item BRIDGE_CHIP = new Item(new Item.Settings());

    @Override
    public void onInitialize() {
        Registry.register(Registries.BLOCK, id("bridge_beacon"), BRIDGE_BEACON);
        Registry.register(Registries.ITEM, id("bridge_beacon"),
                new BlockItem(BRIDGE_BEACON, new Item.Settings()));
        Registry.register(Registries.ITEM, id("bridge_chip"), BRIDGE_CHIP);

        LOGGER.info("[zoro_bridge] ZOROWARRIOR HORIZON BRIDGE loaded for Minecraft {}: "
                + "block zoro_bridge:bridge_beacon, item zoro_bridge:bridge_chip",
                net.fabricmc.loader.api.FabricLoader.getInstance().getModContainer(MOD_ID)
                        .map(container -> container.getMetadata().getVersion().getFriendlyString())
                        .orElse("unknown"));
    }

    public static Identifier id(String path) {
        return Identifier.of(MOD_ID, path);
    }

    private BridgeBeaconMod() {
    }
}
