package dev.zorowarrior.bridge;

import net.fabricmc.api.ModInitializer;
import net.fabricmc.fabric.api.itemgroup.v1.ItemGroupEvents;
import net.minecraft.block.AbstractBlock;
import net.minecraft.block.Block;
import net.minecraft.item.BlockItem;
import net.minecraft.item.Item;
import net.minecraft.item.ItemGroups;
import net.minecraft.registry.Registries;
import net.minecraft.registry.Registry;
import net.minecraft.util.Identifier;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * ZOROWARRIOR HORIZON BRIDGE — the Java Edition half.
 *
 * <p>This is a real, installable mod: it registers one block and one item, with
 * models, textures, a recipe, a tag and language entries, and it announces
 * itself on load. It exists to be an honest counterpart to the bridge: the same
 * jar is the input fixture the Rust tool converts, so what the tool reports
 * about a Java mod is tested against a mod that really exists.
 *
 * <p>Everything in here is server-safe: the block and item are registered on the
 * common initialiser, not the client one, so a dedicated server loads the same
 * content. Rendering is data-only (models and textures), which is what makes the
 * content portable at all.
 */
public final class BridgeBeaconMod implements ModInitializer {
    public static final String MOD_ID = "zoro_bridge";
    public static final Logger LOGGER = LoggerFactory.getLogger(MOD_ID);

    /** The block. Plain stone-like behaviour: hardness, resistance, a light level. */
    public static final Block BRIDGE_BEACON = new Block(
            AbstractBlock.Settings.create()
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

        ItemGroupEvents.modifyEntriesEvent(ItemGroups.BUILDING_BLOCKS)
                .register(entries -> entries.add(BRIDGE_BEACON));
        ItemGroupEvents.modifyEntriesEvent(ItemGroups.INGREDIENTS)
                .register(entries -> entries.add(BRIDGE_CHIP));

        LOGGER.info("ZOROWARRIOR HORIZON BRIDGE loaded: block zoro_bridge:bridge_beacon, "
                + "item zoro_bridge:bridge_chip");
    }

    public static Identifier id(String path) {
        return Identifier.of(MOD_ID, path);
    }

    private BridgeBeaconMod() {
    }
}
