package dev.zorowarrior.bridge;

import java.util.Optional;

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
 * ZOROWARRIOR HORIZON BRIDGE — Fabric, Minecraft 1.21.3 and newer 1.21.x.
 *
 * <p>Vanilla classes only, no Fabric API, and that is a deliberate design
 * constraint rather than a limitation. Two things changed in 1.21.3, both of
 * which only show up when a server actually boots:
 *
 * <ol>
 *   <li>{@code AbstractBlock.Settings} must be given the block's registry key
 *       ({@code registryKey(...)}) before the {@code Block} is constructed, or
 *       the constructor throws {@code NullPointerException: Block id not set}
 *       while resolving a default loot table.</li>
 *   <li>But setting that key makes the registry create an <i>intrusive</i>
 *       holder, which a frozen registry refuses outside the boot phase:
 *       {@code IllegalStateException: This registry can't create intrusive
 *       holders}. Fabric API's old {@code RegistryEvent} was the escape hatch;
 *       it has since been removed, because vanilla now covers it.</li>
 * </ol>
 *
 * <p>The way out that needs no library at all: don't let the block compute a
 * loot table from its id in the first place. Supplying the loot table
 * explicitly satisfies the constructor, so no key is needed, so no intrusive
 * holder is created, so registering from {@code onInitialize} is legal again.
 * The drop is defined by {@code data/zoro_bridge/loot_table/blocks/...}.
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
                // Explicit (empty) loot table: removes the constructor's need to
                // derive one from a registry key it does not have yet.
                .lootTable(Optional.empty())
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
        LOGGER.info("[ZoroHorizonBridge] mod {} loaded on {}: bridge_beacon + bridge_chip registered",
                version, net.fabricmc.loader.api.FabricLoader.getInstance().getEnvironmentType());
    }
}
